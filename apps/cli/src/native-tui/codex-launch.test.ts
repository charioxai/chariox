import assert from "node:assert/strict"
import { once } from "node:events"
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { createRequire } from "node:module"
import test from "node:test"
import { WebSocketServer } from "ws"

import { LocalIpcClient } from "../ipc.js"
import { runCodexNativeTui } from "./codex.js"

// MP-08 / MP-10: Exercise the actual native entry point and proxy. A bare
// server has no granted tool; a kernel-managed launch materializes the grant.
test("MP-08 MP-10 native Codex discovers and invokes the kernel-granted MCP", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-codex-launch-"))
  const servers: WebSocketServer[] = []
  const launches: Record<string, unknown>[] = []
  let calls = 0
  const server = async (granted: boolean, port = 0) => {
    const ws = new WebSocketServer({ host: "127.0.0.1", port })
    servers.push(ws)
    ws.on("connection", (socket) => socket.on("message", (raw) => {
      const request = JSON.parse(raw.toString())
      if (request.id === undefined) return
      let result: unknown
      switch (request.method) {
        case "initialize": result = { userAgent: "fixture" }; break
        case "thread/start": result = { thread: { id: "native-thread" }, model: "fixture" }; break
        case "mcpServerStatus/list": result = { data: granted ? [{ name: "chariox_mcp_echo", tools: { echo_marker: {} } }] : [] }; break
        case "mcpServer/tool/call":
          assert.equal(granted, true)
          calls += 1
          result = { content: [{ type: "text", text: request.params.arguments.marker }] }
          break
        default: throw new Error(`unexpected app-server request ${request.method}`)
      }
      socket.send(JSON.stringify({ id: request.id, result }))
    }))
    await once(ws, "listening")
    const address = ws.address()
    assert.ok(address && typeof address !== "string")
    return `ws://127.0.0.1:${address.port}`
  }
  const endpoint = await server(true)
  const agent = { id: "agent", session_id: "session", alias: "fixture", extension_grants: [{ kind: "mcp", name: "echo" }] }
  const run = { id: "run", session_id: "session", agent_instance_id: "agent", state: "Running", provider: "codex", model: "fixture", structured_endpoint: endpoint, provider_session_id: null, endpoint_mode: "Managed" }
  t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, Record<string, unknown>>) => {
    const [kind, payload] = Object.entries(request)[0]!
    switch (kind) {
      case "CreateSession": return { SessionCreated: { session: { id: "session", alias: "fixture", workspace_id: root, worktree_id: root, agents: [agent] }, agent } }
      case "AttachToSession": return { SessionAttached: { attachment: { id: "attachment" } } }
      case "GrantAgentExtension": return { AgentExtensionGranted: { agent } }
      case "LaunchProviderRun":
        launches.push(payload)
        return { ProviderRunLaunched: { provider_run: { ...run, provider_session_id: payload.provider_session_id ?? null } } }
      case "GetProviderRun": return { ProviderRun: { provider_run: { ...run, provider_session_id: "native-thread" } } }
      case "GetSessionState": return { SessionState: { agent_activity: {} } }
      case "PollRuntimeNotices": return { RuntimeNotices: { notices: [] } }
      case "PumpTerminalOutput": return { TerminalOutput: { records: [] } }
      case "RunShellCommand": {
        // The former local --server-in-kernel path starts a bare server.
        const command = (payload.args as string[])[1] ?? ""
        const listen = command.match(/--listen '(ws:\/\/[^']+)'/)?.[1]
        if (listen) await server(false, Number(new URL(listen).port))
        return { ShellCommandCompleted: { result: { exit_code: 0, stdout: "fixture-pid", stderr: "" } } }
      }
      default: throw new Error(`unexpected kernel request ${kind}`)
    }
  })
  t.mock.method(LocalIpcClient.prototype, "close", async () => {})
  const executable = path.join(root, "codex-fixture.mjs")
  const wsModule = createRequire(import.meta.url).resolve("ws")
  await writeFile(executable, `#!/usr/bin/env node
import { createRequire } from 'node:module';
const WebSocket = createRequire(import.meta.url)(${JSON.stringify(wsModule)});
const endpoint = process.argv[process.argv.indexOf('--remote') + 1];
const socket = new WebSocket(endpoint);
let next = 1; const pending = new Map();
socket.on('message', raw => { const r = JSON.parse(raw.toString()); const p = pending.get(r.id); if (p) { pending.delete(r.id); r.error ? p.reject(new Error(r.error.message)) : p.resolve(r.result); } });
const request = (method, params) => new Promise((resolve, reject) => { const id = next++; pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })); });
try {
await new Promise((resolve, reject) => { socket.once('open', resolve); socket.once('error', reject); });
await request('initialize', { clientInfo: { name: 'codex-tui', version: 'fixture' } });
await request('thread/start', {});
const status = await request('mcpServerStatus/list', { threadId: 'native-thread' });
if (!status.data.some(s => s.tools.echo_marker)) throw new Error('kernel-granted echo_marker was not discovered');
const result = await request('mcpServer/tool/call', { threadId: 'native-thread', server: 'chariox_mcp_echo', tool: 'echo_marker', arguments: { marker: 'MP08-MP10' } });
if (result.content[0].text !== 'MP08-MP10') throw new Error('marker invocation failed');
} catch (error) { process.stderr.write(error.message + '\\n'); process.exitCode = 1; }
finally { socket.close(); }
`)
  await chmod(executable, 0o755)
  const previous = process.env.CHARIOX_CODEX_BIN
  process.env.CHARIOX_CODEX_BIN = executable
  try {
    for (const compatibilityFlags of [["--server-in-kernel"], []]) {
      await runCodexNativeTui(["--workspace", root, "--worktree", root, ...compatibilityFlags, "--grant-mcp", "echo", "--model", "fixture"])
    }
    assert.equal(calls, 2)
    assert.equal(launches.length, 2)
    assert.ok(launches.every((launch) => launch.structured_endpoint === null))
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_CODEX_BIN
    else process.env.CHARIOX_CODEX_BIN = previous
    for (const ws of servers) {
      for (const socket of ws.clients) socket.terminate()
      await new Promise<void>((resolve) => ws.close(() => resolve()))
    }
    await rm(root, { recursive: true, force: true })
  }
})
