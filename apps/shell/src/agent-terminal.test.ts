import assert from "node:assert/strict"
import { Readable, Writable } from "node:stream"
import test from "node:test"

import { abortInFlightRequests, handleRequest, resolveAgentTerminalConnection, runAgentTerminalJsonl, runAgentTerminalServer } from "./agent-terminal.js"

test("agent terminal remote bootstrap follows the home-kernel connection contract", async () => {
  const requests: Record<string, unknown>[] = []
  let closed = false
  const connection = await resolveAgentTerminalConnection({
    endpoint: "ws://unused",
    homeKernelEndpoint: "ws://home",
    targetKernelRef: "kernel-remote",
    targetMachineRef: "machine-remote",
    targetSessionId: "session-remote",
    clientId: "agent-client",
    bootstrapClient: {
      send: async (request) => {
        requests.push(request)
        return {
          KernelClientConnectionResolved: {
            connection: {
              relay_url: "wss://relay.example/kernel",
              relay_token: "short-lived-token",
              target_daemon_id: "daemon-remote",
              target_daemon_alias: "remote",
            },
          },
        }
      },
      close: async () => { closed = true },
    },
  })
  assert.deepEqual(requests, [{
    ResolveKernelClientConnection: {
      kernel_ref: "kernel-remote",
      machine_ref: "machine-remote",
      client_id: "agent-client",
      session_id: "session-remote",
    },
  }])
  assert.equal(connection.endpoint, "wss://relay.example/kernel")
  assert.equal(connection.relayAuthToken, "short-lived-token")
  assert.equal(connection.targetDaemonId, "daemon-remote")
  assert.equal(connection.targetDaemonAlias, "remote")
  assert.equal(await connection.relayAuthTokenProvider?.(), "short-lived-token")
  assert.equal(requests.length, 2, "remote token refresh resolves through the home kernel")
  assert.equal(closed, false, "injected bootstrap clients remain caller-owned")
})

test("agent terminal remote bootstrap rejects partial target configuration", async () => {
  await assert.rejects(
    () => resolveAgentTerminalConnection({ endpoint: "ws://unused", targetKernelRef: "kernel-remote" }),
    /requires both homeKernelEndpoint and targetKernelRef/,
  )
})

test("agent terminal MCP handshake exposes the five stable tools", async () => {
  const terminal = {
    search: async () => ({ revision: "r1", results: [] }),
    describe: async () => ({ revision: "r1", command: { id: "session-list" } }),
    execute: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    wait: async () => ({ completed: true, timed_out: false, session: null }),
  } as never
  const initialize = await handleRequest(terminal, { jsonrpc: "2.0", id: 1, method: "initialize" })
  assert.equal((initialize?.result as { protocolVersion: string }).protocolVersion, "2024-11-05")
  assert.match((initialize?.result as { instructions: string }).instructions, /chariox_search/)
  assert.match((initialize?.result as { instructions: string }).instructions, /explicit workspace, worktree/)
  const listed = await handleRequest(terminal, { jsonrpc: "2.0", id: 2, method: "tools/list" })
  const listedTools = (listed?.result as { tools: Array<{ name: string; inputSchema?: { properties?: Record<string, unknown> } }> }).tools
  const names = listedTools.map((tool) => tool.name)
  assert.deepEqual(names, ["chariox_search", "chariox_describe", "chariox_execute", "chariox_wait", "chariox_status"])
  const executeContext = listedTools.find((tool) => tool.name === "chariox_execute")?.inputSchema?.properties?.context as { properties?: Record<string, unknown> } | undefined
  assert.deepEqual(executeContext?.properties?.workspace_id, { type: ["string", "null"] })
  assert.deepEqual(executeContext?.properties?.worktree_id, { type: ["string", "null"] })
  const search = await handleRequest(terminal, {
    jsonrpc: "2.0",
    id: 3,
    method: "tools/call",
    params: { name: "chariox.search", arguments: { query: "workflow", limit: 5 } },
  })
  assert.equal(search?.id, 3)
  assert.equal((search?.result as { isError: boolean }).isError, false)
  assert.equal(await handleRequest(terminal, { jsonrpc: "2.0", method: "notifications/cancelled" }), null)
  assert.equal(await handleRequest(terminal, { jsonrpc: "2.0", method: "ping" }), null)
})

test("agent terminal MCP rejects unknown methods without dispatching", async () => {
  const terminal = {
    search: async () => ({ revision: "r1", results: [] }),
    describe: async () => ({ revision: "r1", command: { id: "session-list" } }),
    execute: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    executeOperation: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    wait: async () => ({ completed: true, timed_out: false, session: null }),
    status: async () => ({ connected: true, context: { workspace: "/repo", worktree: "/repo" }, session: null, registry_revision: "r1" }),
  } as never
  const response = await handleRequest(terminal, { jsonrpc: "2.0", id: 24, method: "not-a-method" })
  assert.equal(response?.error && (response.error as { code?: number }).code, -32601)
  assert.match(String((response?.error as { message?: string }).message), /method not found/)
})

test("agent terminal cancellation notifications identify the in-flight request", async () => {
  let cancelled: string | number | undefined
  const terminal = {
    search: async () => ({ revision: "r1", results: [] }),
    describe: async () => ({ revision: "r1", command: { id: "session-list" } }),
    execute: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    wait: async () => ({ completed: true, timed_out: false, session: null }),
  } as never
  assert.equal(
    await handleRequest(terminal, { jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 42 } }, { cancel: (id) => { cancelled = id } }),
    null,
  )
  assert.equal(cancelled, 42)
})

test("agent terminal MCP validates bounded arguments before dispatch", async () => {
  const terminal = {
    search: async () => ({ revision: "r1", results: [] }),
    describe: async () => ({ revision: "r1", command: { id: "session-list" } }),
    execute: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    executeOperation: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    wait: async () => ({ completed: true, timed_out: false, session: null }),
    status: async () => ({ connected: true, context: { workspace: "/repo", worktree: "/repo" }, session: null, registry_revision: "r1" }),
  } as never
  const badSearch = await handleRequest(terminal, { jsonrpc: "2.0", id: 21, method: "tools/call", params: { name: "chariox_search", arguments: { limit: 0 } } })
  assert.equal((badSearch?.result as { isError: boolean }).isError, true)
  const badExecute = await handleRequest(terminal, { jsonrpc: "2.0", id: 22, method: "tools/call", params: { name: "chariox_execute", arguments: { command: "pwd", operation_id: "terminal.list_sessions", context: { workspace: "/repo", worktree: "/repo" } } } })
  assert.equal((badExecute?.result as { isError: boolean }).isError, true)
})

test("agent terminal adapters forward an expected registry revision", async () => {
  let observedRevision: string | undefined
  const terminal = {
    search: async () => ({ revision: "r1", results: [] }),
    describe: async () => ({ revision: "r1", command: { id: "session-list" } }),
    execute: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" }, registry_revision: "r1" }),
    executeOperation: async (_operationId: string, _input: unknown, _context: unknown, options: { registry_revision?: string }) => {
      observedRevision = options.registry_revision
      return { ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" }, registry_revision: "r1" }
    },
    wait: async () => ({ completed: true, timed_out: false, session: null }),
    status: async () => ({ connected: true, context: { workspace: "/repo", worktree: "/repo" }, session: null, registry_revision: "r1" }),
  } as never
  const response = await handleRequest(terminal, {
    jsonrpc: "2.0",
    id: 23,
    method: "tools/call",
    params: {
      name: "chariox_execute",
      arguments: {
        operation_id: "terminal.list_sessions",
        registry_revision: "r1",
        context: { workspace: "/repo", worktree: "/repo" },
      },
    },
  })
  assert.equal((response?.result as { isError: boolean }).isError, false)
  assert.equal(observedRevision, "r1")
})

test("agent terminal aborts outstanding requests when the input stream reaches EOF", () => {
  const controller = new AbortController()
  abortInFlightRequests(new Map([[10, controller]]))
  assert.equal(controller.signal.aborted, true)
})

test("agent terminal server settles a blocked wait when input reaches EOF", async () => {
  const chunks: string[] = []
  const output = new Writable({ write(chunk, _encoding, callback) { chunks.push(String(chunk)); callback() } })
  const client = {
    send: async (request: Record<string, unknown>) => {
      if ("GetSessionState" in request) {
        return { SessionState: { session: { agent_activity: { "agent-1": { busy: true, status: "working" } } } } }
      }
      return {}
    },
    close: async () => {},
  }
  const started = Date.now()
  await runAgentTerminalServer({
    endpoint: "ws://unused",
    input: Readable.from([JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/call", params: { name: "chariox.wait", arguments: { context: { workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a", agent_id: "agent-1" } } } })]),
    output,
    client,
  })
  assert.ok(Date.now() - started < 1000)
  assert.match(chunks.join(""), /"id":7/)
  assert.match(chunks.join(""), /isError/)
})

test("agent terminal request handling keeps pings independent of a slow wait", async () => {
  let releaseWait: (() => void) | undefined
  const wait = new Promise<void>((resolve) => { releaseWait = resolve })
  const terminal = {
    search: async () => ({ revision: "r1", results: [] }),
    describe: async () => ({ revision: "r1", command: { id: "session-list" } }),
    execute: async () => ({ ok: true, output: "ok", result: null, context: { workspace: "/repo", worktree: "/repo" } }),
    wait: async () => { await wait; return { completed: true, timed_out: false, session: null } },
  } as never
  const blocked = handleRequest(terminal, { jsonrpc: "2.0", id: 10, method: "tools/call", params: { name: "chariox.wait", arguments: { context: { workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a", agent_id: "agent-1" } } } })
  const ping = await handleRequest(terminal, { jsonrpc: "2.0", id: 11, method: "ping" })
  assert.equal(ping?.id, 11)
  releaseWait?.()
  assert.equal((await blocked)?.id, 10)
})

test("agent terminal JSONL adapter exposes the same stateless operations", async () => {
  const chunks: string[] = []
  const output = new Writable({ write(chunk, _encoding, callback) { chunks.push(String(chunk)); callback() } })
  const client = {
    send: async (request: Record<string, unknown>) => {
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: { revision: "r1", operations: [] } } }
      }
      return {}
    },
    close: async () => {},
  }
  await runAgentTerminalJsonl({
    endpoint: "ws://unused",
    input: Readable.from(`${JSON.stringify({ id: 1, op: "search", query: "workflow" })}\n${JSON.stringify({ id: 2, op: "status", context: { workspace: "/repo", worktree: "/repo" } })}\n`),
    output,
    client,
  })
  const responses = chunks.join("").trim().split("\n").map((line) => JSON.parse(line) as { id: number; ok: boolean })
  assert.deepEqual(responses.map((response) => [response.id, response.ok]), [[1, true], [2, true]])
})

test("agent terminal JSONL adapter reports malformed JSON and unknown operations", async () => {
  const chunks: string[] = []
  const output = new Writable({ write(chunk, _encoding, callback) { chunks.push(String(chunk)); callback() } })
  const client = {
    send: async () => ({}),
    close: async () => {},
  }
  await runAgentTerminalJsonl({
    endpoint: "ws://unused",
    input: Readable.from(`{malformed\n${JSON.stringify({ id: 3, op: "not-an-operation" })}\n`),
    output,
    client,
  })
  const responses = chunks.join("").trim().split("\n").map((line) => JSON.parse(line) as { id: number | null; ok: boolean; error?: string })
  assert.equal(responses.length, 2)
  assert.equal(responses[0]?.id, null)
  assert.equal(responses[0]?.ok, false)
  assert.match(responses[0]?.error ?? "", /Unexpected token|JSON/)
  assert.deepEqual(responses[1], { id: 3, ok: false, error: "unknown agent terminal operation: not-an-operation" })
})

test("agent terminal JSONL adapter aborts a blocked wait when input reaches EOF", async () => {
  const chunks: string[] = []
  const output = new Writable({ write(chunk, _encoding, callback) { chunks.push(String(chunk)); callback() } })
  const client = {
    send: async (request: Record<string, unknown>) => {
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: { revision: "r1", operations: [] } } }
      }
      if ("GetSessionState" in request) {
        return { SessionState: { session: { agent_activity: { "agent-1": { busy: true, status: "working" } } } } }
      }
      return {}
    },
    close: async () => {},
  }
  const started = Date.now()
  await runAgentTerminalJsonl({
    endpoint: "ws://unused",
    input: Readable.from(`${JSON.stringify({ id: 7, op: "wait", timeout_ms: 120000, context: { workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a", agent_id: "agent-1" } })}\n`),
    output,
    client,
  })
  assert.ok(Date.now() - started < 1000)
  assert.match(chunks.join(""), /"id":7/)
  assert.match(chunks.join(""), /"ok":false/)
})
