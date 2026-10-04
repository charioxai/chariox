import assert from "node:assert/strict"
import { once } from "node:events"
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { createRequire } from "node:module"
import test, { type TestContext } from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import { LocalIpcClient } from "../ipc.js"
import { runCodexNativeTui } from "./codex.js"
import { waitForNativeProviderRunReady } from "./provider-run-control.js"

// MP-08 / MP-10: actual entry point + proxy with distinct display/managed turns.
async function fixture(t: TestContext, mode: "interrupt" | "cold" | "ended" | "steer" | "compact" | "controls" | "stale") {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-codex-lifecycle-"))
  const reservation = net.createServer().listen(0, "127.0.0.1")
  await once(reservation, "listening")
  const address = reservation.address()
  assert.ok(address && typeof address !== "string")
  await new Promise<void>((resolve) => reservation.close(() => resolve()))
  const endpoint = `ws://127.0.0.1:${address.port}`
  let server: WebSocketServer | undefined
  let managed: WebSocket | undefined
  let running = false
  let polls = 0
  let tuiStarted = false
  const interrupts: unknown[] = []
  const controls: { method: string, params: Record<string, unknown> }[] = []
  let compacted = false
  const promptContexts: boolean[] = []
  const startServer = async () => {
    server = new WebSocketServer({ host: "127.0.0.1", port: address.port })
    server.on("connection", (socket) => socket.on("message", (raw) => {
      const r = JSON.parse(raw.toString())
      let result: unknown = {}
      if (r.method === "initialize") { tuiStarted = true; result = { userAgent: "fixture" } }
      if (r.method === "thread/start") result = { thread: { id: "display-thread" } }
      if (r.method === "turn/start") {
        running = true
        promptContexts.push(compacted)
        result = { turn: { id: "managed-turn" } }
      }
      if (r.method === "thread/read") result = { thread: { id: r.params.threadId, turns: running ? [{ id: "managed-turn", status: "inProgress" }] : [] } }
      if (r.method === "turn/steer" || r.method === "thread/compact/start") {
        controls.push({ method: r.method, params: r.params })
        if (r.params.threadId !== "managed-thread" || (r.method === "turn/steer" && r.params.expectedTurnId !== "managed-turn")) {
          socket.send(JSON.stringify({ id: r.id, error: { code: -32000, message: "control missed managed conversation" } }))
          return
        }
        if (r.method === "thread/compact/start") { compacted = true; running = false }
        result = r.method === "turn/steer" ? { turnId: "managed-turn" } : {}
      }
      if (r.method === "turn/interrupt") {
        interrupts.push(r.params)
        if (r.params.threadId === "managed-thread" && r.params.turnId === "managed-turn") running = false
      }
      socket.send(JSON.stringify({ id: r.id, result }))
    }))
    await once(server, "listening")
  }
  if (!["cold", "ended"].includes(mode)) await startServer()
  let nextId = 1
  const managedRequest = async (method: string, params: unknown) => {
    if (!managed) { managed = new WebSocket(endpoint); await once(managed, "open") }
    const id = nextId++
    const received = once(managed, "message")
    managed.send(JSON.stringify({ id, method, params }))
    await received
  }
  const agent = { id: "agent", session_id: "session", alias: "fixture" }
  const run = { id: "run", session_id: "session", agent_instance_id: "agent", provider: "codex", structured_endpoint: endpoint, provider_session_id: null }
  t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, Record<string, unknown>>) => {
    const [kind, payload] = Object.entries(request)[0]!
    switch (kind) {
      case "CreateSession": return { SessionCreated: { session: { id: "session", alias: "fixture", workspace_id: root, worktree_id: root, agents: [agent] }, agent } }
      case "AttachToSession": return { SessionAttached: { attachment: { id: "attachment" } } }
      case "LaunchProviderRun": return { ProviderRunLaunchAccepted: { provider_run: { ...run, state: mode === "cold" || mode === "ended" ? "Starting" : "Running" } } }
      case "GetProviderRun":
        polls += 1
        if (mode === "cold" && polls === 2) await startServer()
        return { ProviderRun: { provider_run: { ...run, provider_session_id: running || compacted ? "managed-thread" : null, state: mode === "ended" ? "Ended" : mode === "cold" && polls < 2 ? "Starting" : "Running" } } }
      case "SubmitPrompt":
        assert.equal(payload.target_agent_id, "agent")
        await managedRequest("turn/start", { threadId: "managed-thread", input: [{ type: "text", text: payload.prompt }] })
        return { PromptSubmitted: { outcome: { Started: { prompt: { id: "home-prompt", target_agent_id: "agent" } } } } }
      case "SteerActivePrompt":
        assert.deepEqual(payload, {
          session_id: "session", attachment_id: "attachment", target_agent_id: "agent",
          expected_active_prompt_id: "home-prompt", prompt: "change direction\n", attachments: [],
        })
        await managedRequest("turn/steer", { threadId: "managed-thread", expectedTurnId: "managed-turn", input: [{ type: "text", text: payload.prompt }] })
        return { ActivePromptSteered: { prompt: { id: "home-steer" } } }
      case "CancelActivePrompt":
        assert.deepEqual(payload, { session_id: "session", attachment_id: "attachment", target_agent_id: "agent" })
        await managedRequest("turn/interrupt", { threadId: "managed-thread", turnId: "managed-turn" })
        return { PromptCancelled: {} }
      case "PumpTerminalOutput": return { TerminalOutput: { records: [] } }
      default: throw new Error(`unexpected kernel request ${kind}`)
    }
  })
  t.mock.method(LocalIpcClient.prototype, "close", async () => {})
  const executable = path.join(root, "codex-fixture.mjs")
  await writeFile(executable, `#!/usr/bin/env node
import { createRequire } from 'node:module';
const WebSocket = createRequire(import.meta.url)(${JSON.stringify(createRequire(import.meta.url).resolve("ws"))});
const socket = new WebSocket(process.argv[process.argv.indexOf('--remote') + 1]);
let next = 1; const pending = new Map();
socket.on('message', raw => { const r = JSON.parse(raw.toString()); const p = pending.get(r.id); if (p) { pending.delete(r.id); r.error ? p.reject(new Error(r.error.message)) : p.resolve(r.result); } });
socket.on('close', () => { for (const p of pending.values()) p.reject(new Error('connection closed before response')); });
const request = (method, params) => new Promise((resolve, reject) => { const id = next++; pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })); });
const timer = setTimeout(() => { process.stderr.write('fixture timed out\\n'); process.exit(1); }, 3000);
try {
await new Promise((resolve, reject) => { socket.once('open', resolve); socket.once('error', reject); });
await request('initialize', { clientInfo: { name: 'codex-tui', version: 'fixture' } });
await request('thread/start', {});
if (['interrupt', 'steer', 'compact', 'controls', 'stale'].includes(${JSON.stringify(mode)})) {
 const result = await request('turn/start', { threadId: 'display-thread', input: [{ type: 'text', text: 'long prompt' }] });
 if (${JSON.stringify(mode)} === 'stale') {
   for (const params of [
     { threadId: 'display-thread', expectedTurnId: 'unknown-turn' },
     { threadId: 'other-thread', expectedTurnId: result.turn.id },
   ]) {
     try { await request('turn/steer', { ...params, input: [{ type: 'text', text: 'change direction' }] }); throw new Error('stale steer was admitted'); }
     catch (error) { if (!error.message.includes('not bound')) throw error; }
   }
 }
 if (${JSON.stringify(mode)} === 'interrupt') await request('turn/interrupt', { threadId: 'display-thread', turnId: result.turn.id });
 if (['steer', 'controls'].includes(${JSON.stringify(mode)})) {
   const response = await request('turn/steer', { threadId: 'display-thread', expectedTurnId: result.turn.id, input: [{ type: 'text', text: 'change direction' }] });
   if (response.turnId !== result.turn.id) throw new Error('steer response exposed the execution turn');
 }
 if (['compact', 'controls'].includes(${JSON.stringify(mode)})) {
   await request('thread/compact/start', { threadId: 'display-thread' });
   await request('turn/start', { threadId: 'display-thread', input: [{ type: 'text', text: 'after compaction' }] });
 }
}
} catch (error) { process.stderr.write(error.message + '\\n'); process.exitCode = 1; }
finally { clearTimeout(timer); socket.close(); }
`)
  await chmod(executable, 0o755)
  const previous = process.env.CHARIOX_CODEX_BIN
  process.env.CHARIOX_CODEX_BIN = executable
  try {
    const launch = () => runCodexNativeTui(["--workspace", root, "--worktree", root, "--model", "fixture"])
    if (mode === "ended") {
      await assert.rejects(launch, /provider run ended before.*ready/)
      assert.equal(tuiStarted, false)
    } else {
      await launch()
      if (mode === "interrupt") {
        assert.equal(running, false, "interrupt must stop the managed turn, not the synthetic display turn")
        assert.deepEqual(interrupts, [{ threadId: "managed-thread", turnId: "managed-turn" }])
      } else if (mode === "steer" || mode === "compact" || mode === "controls") {
        const expected = []
        if (mode !== "compact") expected.push({ method: "turn/steer", params: { threadId: "managed-thread", expectedTurnId: "managed-turn", input: [{ type: "text", text: "change direction\n" }] } })
        if (mode !== "steer") expected.push({ method: "thread/compact/start", params: { threadId: "managed-thread" } })
        assert.deepEqual(controls, expected)
        assert.deepEqual(promptContexts, mode === "steer" ? [false] : [false, true], "subsequent prompts must use the compacted managed conversation")
      } else if (mode === "stale") assert.deepEqual(controls, [], "unknown display identities must never reach the provider")
      else assert.ok(polls >= 2, "must wait for asynchronous launch completion before starting the TUI")
    }
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_CODEX_BIN
    else process.env.CHARIOX_CODEX_BIN = previous
    managed?.terminate()
    if (server) {
      for (const socket of server.clients) socket.terminate()
      await new Promise<void>((resolve) => server!.close(() => resolve()))
    }
    await rm(root, { recursive: true, force: true })
  }
}

test("MP-08 MP-10 native entry point interrupts the kernel managed turn", (t) => fixture(t, "interrupt"))
test("MP-08 MP-10 native entry point waits for a cold managed endpoint", (t) => fixture(t, "cold"))
test("MP-08 MP-10 native entry point reports an ended launch before attaching", (t) => fixture(t, "ended"))

test("MP-08 MP-10 native entry point steers the kernel managed turn", (t) => fixture(t, "steer"))
test("MP-08 MP-10 native entry point compacts the conversation used for subsequent prompts", (t) => fixture(t, "compact"))
test("MP-08 MP-10 native entry point steers then compacts the managed conversation", (t) => fixture(t, "controls"))
test("MP-08 MP-10 native entry point rejects stale turn and foreign thread steering", (t) => fixture(t, "stale"))

// MP-08 / MP-10: A launch that never settles has a bounded actionable failure.
test("MP-08 MP-10 managed endpoint readiness timeout names the pending run", async () => {
  const client = { send: async () => ({ ProviderRun: { provider_run: { id: "pending-run", state: "Starting" } } }) } as unknown as LocalIpcClient
  await assert.rejects(waitForNativeProviderRunReady(client, "pending-run", { timeoutMs: 20, pollIntervalMs: 1 }),
    /timed out.*pending-run \(Starting\)/)
})

test("MP-08 MP-10 managed endpoint readiness also bounds a stalled status RPC", async () => {
  const client = { send: async () => new Promise(() => {}) } as unknown as LocalIpcClient
  await assert.rejects(waitForNativeProviderRunReady(client, "stalled-run", { timeoutMs: 20 }),
    /timed out.*stalled-run \(unknown\)/)
})
