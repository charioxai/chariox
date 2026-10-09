import assert from "node:assert/strict"
import { once } from "node:events"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import { startCodexProxy } from "./codex-proxy.js"
import type { LocalIpcClient } from "../ipc.js"
import type { RuntimeProviderRun } from "../cli-types.js"

test("MP-08/MP-10/MP-11 native display lifecycle excludes managed execution notifications", async () => {
  const upstream = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await once(upstream, "listening")
  const address = upstream.address()
  assert(address && typeof address !== "string")
  const run = { id: "run", session_id: "session", agent_instance_id: "agent", provider: "codex", state: "Running", structured_endpoint: `ws://127.0.0.1:${address.port}`, provider_session_id: "managed-thread" } as RuntimeProviderRun
  let managedSubscribed = false
  let providerSocket: WebSocket | undefined
  upstream.on("connection", (socket) => {
    providerSocket = socket
    socket.on("message", (data) => {
      const message = JSON.parse(data.toString())
      if (message.method === "thread/resume") {
        assert.deepEqual(message.params, { threadId: "managed-thread", excludeTurns: true }, "subscribe only the owned running thread and exclude history")
        managedSubscribed = true
      }
      socket.send(JSON.stringify({ id: message.id, result: message.method === "thread/start" ? { thread: { id: "display-thread" } } : message.method === "thread/resume" ? {thread: {id: "managed-thread"}} : {} }))
    })
  })
  const proxy = await startCodexProxy({ upstreamEndpoint: run.structured_endpoint!, sessionId: "session", attachmentId: "attachment", agentId: "agent", model: "gpt-6.1-sol", effort: "low", bindState: { run, promise: Promise.resolve(run) }, inlineLocalAttachments: false, debug: () => {}, client: { send: async () => ({ ProviderRun: {provider_run: run} }) } as unknown as LocalIpcClient })
  const proxyAddress = proxy.address()
  assert(proxyAddress && typeof proxyAddress !== "string")
  const tui = new WebSocket(`ws://127.0.0.1:${proxyAddress.port}`)
  const messages: Array<{ id?: unknown; method?: string; error?: unknown; params?: { threadId?: string } }> = []
  tui.on("message", (data) => messages.push(JSON.parse(data.toString())))
  const waitFor = async (predicate: () => boolean) => {
    for (let i = 0; i < 100; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 10)) }
    assert.fail("MP-08 proxy response deadline")
  }
  try {
    await once(tui, "open")
    tui.send(JSON.stringify({ id: 1, method: "initialize", params: { clientInfo: { name: "codex_cli_rs" } } }))
    await waitFor(() => messages.some(message => message.id === 1))
    tui.send(JSON.stringify({ id: 2, method: "thread/start", params: {} }))
    await waitFor(() => messages.some(message => message.id === 2))
    for (const method of ["thread/status/changed", "turn/started", "item/agentMessage/delta", "turn/completed"]) {
      providerSocket!.send(JSON.stringify({ method, params: { threadId: "managed-thread", turnId: "managed-turn", turn: { id: "managed-turn" }, delta: "execution output" } }))
    }
    for (const type of ["task_started", "agent_message", "task_complete"]) {
      providerSocket!.send(JSON.stringify({ method: `codex/event/${type}`, params: { conversationId: "managed-thread", id: "managed-turn", msg: { type } } }))
    }
    providerSocket!.send(JSON.stringify({ id: 77, method: "item/commandExecution/requestApproval", params: { threadId: "managed-thread", turnId: "managed-turn" } }))
    providerSocket!.send(JSON.stringify({ method: "account/rateLimits/updated", params: {} }))
    await waitFor(() => messages.some(message => message.method === "account/rateLimits/updated"))
    assert.equal(messages.filter(message => message.id === undefined && message.params?.threadId === "managed-thread").length, 0, "managed lifecycle must reach the display through the home prompt projection")
    assert.equal(messages.filter(message => message.method?.startsWith("codex/event/")).length, 0, "legacy managed lifecycle cannot replace the home prompt identity")
    assert(messages.some(message => message.id === 77), "native server requests remain available for approval")
    tui.send(JSON.stringify({ id: 4, method: "thread/compact/start", params: { threadId: "foreign-display" } }))
    await waitFor(() => messages.some(message => message.id === 4))
    assert(messages.find(message => message.id === 4)?.error, "foreign display controls must be rejected")
    tui.send(JSON.stringify({id: 3, method: "thread/compact/start", params: {threadId: "display-thread"}}))
    await waitFor(() => messages.some(message => message.id === 3))
    assert(managedSubscribed, "MP-08 compaction needs the actual managed thread subscription")
    tui.send(JSON.stringify({ id: 5, method: "thread/compact/start", params: { threadId: "display-thread" } }))
    await waitFor(() => messages.some(message => message.id === 5))
    assert(messages.find(message => message.id === 5)?.error, "an active control must not be overwritten")
    providerSocket!.send(JSON.stringify({method: "turn/started", params: {threadId: "managed-thread", turn: {id: "compact-turn"}}}))
    providerSocket!.send(JSON.stringify({ method: "item/completed", params: { threadId: "foreign-thread", turnId: "compact-turn", item: { type: "contextCompaction", id: "foreign-item" } } }))
    providerSocket!.send(JSON.stringify({ method: "item/completed", params: { threadId: "managed-thread", turnId: "foreign-turn", item: { type: "contextCompaction", id: "foreign-item" } } }))
    for (const method of ["item/started", "item/completed"]) providerSocket!.send(JSON.stringify({method, params: {threadId: "managed-thread", turnId: "compact-turn", item: {type: "contextCompaction", id: "compact-item"}}}))
    providerSocket!.send(JSON.stringify({method: "turn/completed", params: {threadId: "managed-thread", turn: {id: "compact-turn", status: "completed"}}}))
    await waitFor(() => messages.some(message => message.method === "item/completed"))
    assert.equal(messages.find(message => message.method === "item/completed")?.params?.threadId, "display-thread", "explicit managed compaction must visibly complete in its requesting display thread")
    assert.equal(messages.find(message => message.method === "turn/completed")?.params?.threadId, "display-thread")
    assert.equal(messages.filter(message => message.method === "item/completed" && message.params?.threadId === "display-thread").length, 1, "foreign thread and turn notifications must not enter the control projection")
    providerSocket!.send(JSON.stringify({method: "turn/started", params: {threadId: "managed-thread", turn: {id: "unrelated-turn"}}}))
    await new Promise(resolve => setTimeout(resolve, 30))
    assert.equal(messages.filter(message => message.method === "turn/started").length, 1, "compaction route ends with its own turn")
    messages.length = 0
    proxy.projectKernelOutputToTui([{ agent_id: "agent", prompt_id: "home-prompt", kind: "provider_output", bytes: [...Buffer.from("visible progress")], timestamp_ms: 1 }])
    await waitFor(() => messages.some(message => message.method === "turn/started"))
    assert.equal(messages.find(message => message.method === "turn/started")?.params?.threadId, "display-thread")
  } finally {
    tui.close(); providerSocket?.close()
    await new Promise<void>(resolve => proxy.close(() => resolve()))
    await new Promise<void>(resolve => upstream.close(() => resolve()))
  }
})
