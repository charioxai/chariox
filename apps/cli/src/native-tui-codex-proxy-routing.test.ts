import assert from "node:assert/strict"
import { once } from "node:events"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import type { LocalIpcClient } from "./ipc.js"
import { startCodexProxy } from "./native-tui/codex-proxy.js"

// MP-08/MP-10: provider utility turns must not appear as Room user prompts.
test("native proxy forwards ephemeral utility turns without binding a Room run", async (t) => {
  const upstream = new WebSocketServer({ port: 0, host: "127.0.0.1" })
  await once(upstream, "listening")
  const address = upstream.address()
  assert.ok(address && typeof address !== "string")
  const requests: Record<string, unknown>[] = []
  upstream.on("connection", (socket) => socket.on("message", (raw) => {
    const message = JSON.parse(raw.toString())
    requests.push(message)
    socket.send(JSON.stringify(message.method === "thread/start"
      ? { method: "thread/started", params: { thread: { id: "utility-thread", ephemeral: true } } }
      : { method: "item/agentMessage/delta", params: { threadId: "utility-thread", delta: "Internal title" } }))
    socket.send(JSON.stringify({ id: message.id, result: message.method === "thread/start"
      ? { thread: { id: "utility-thread" } } : { turn: { id: "utility-turn" } } }))
  }))
  const kernelRequests: unknown[] = []
  const bindState = { promise: null, run: null, structuredEndpoint: "ws://unused" }
  const proxy = await startCodexProxy({
    upstreamEndpoint: `ws://127.0.0.1:${address.port}`,
    client: { send: async (request: unknown) => { kernelRequests.push(request); throw new Error("utility reached kernel") } } as unknown as LocalIpcClient,
    sessionId: "room", attachmentId: "attachment", agentId: "agent", model: "model", effort: "medium",
    bindState, inlineLocalAttachments: false, debug: () => {},
  })
  const proxyAddress = proxy.address()
  assert.ok(proxyAddress && typeof proxyAddress !== "string")
  const kernel = new WebSocket(`ws://127.0.0.1:${proxyAddress.port}`)
  await once(kernel, "open")
  const initialized = once(kernel, "message")
  kernel.send(JSON.stringify({ id: "kernel-init", method: "initialize", params: { clientInfo: { name: "chariox-kernel" } } }))
  await initialized
  const kernelMessages: unknown[] = []
  kernel.on("message", (raw) => kernelMessages.push(JSON.parse(raw.toString())))
  const native = new WebSocket(`ws://127.0.0.1:${proxyAddress.port}`)
  t.after(async () => {
    native.close()
    kernel.close()
    await new Promise<void>((resolve) => proxy.close(() => resolve()))
    await new Promise<void>((resolve) => upstream.close(() => resolve()))
  })
  await once(native, "open")
  const response = async (request: unknown) => {
    const received = new Promise<{ result: { thread?: { id: string }, turn?: { id: string } } }>((resolve) => {
      const listener = (raw: WebSocket.RawData) => {
        const message = JSON.parse(raw.toString())
        if (message.id === (request as { id: unknown }).id) { native.off("message", listener); resolve(message) }
      }
      native.on("message", listener)
    })
    native.send(JSON.stringify(request))
    return received
  }
  assert.equal((await response({ id: 1, method: "thread/start", params: { ephemeral: true } })).result.thread?.id, "utility-thread")
  assert.equal((await response({ id: 2, method: "turn/start", params: { threadId: "utility-thread", input: [{ type: "text", text: "Generate a title" }] } })).result.turn?.id, "utility-turn")
  await response({ id: 3, method: "turn/interrupt", params: { threadId: "utility-thread", turnId: "utility-turn" } })
  await response({ id: 4, method: "turn/steer", params: { threadId: "utility-thread", expectedTurnId: "utility-turn", input: [] } })
  await response({ id: 5, method: "thread/compact/start", params: { threadId: "utility-thread" } })
  assert.deepEqual(requests.map((request) => request.method), ["thread/start", "turn/start", "turn/interrupt", "turn/steer", "thread/compact/start"])
  assert.deepEqual(kernelRequests, [])
  assert.deepEqual(kernelMessages, [])
  assert.equal(bindState.promise, null)
})

for (const origin of ["native", "chariox", "stale", "provider", "pending", "user", "transition"] as const) {
  test(`MP-08/MP-11 native interrupt correlates the ${origin} display turn to its kernel prompt`, { timeout: 10_000 }, async (t) => {
    const upstream = new WebSocketServer({ port: 0, host: "127.0.0.1" })
    await once(upstream, "listening")
    const address = upstream.address()
    assert.ok(address && typeof address !== "string")
    const upstreamRequests: string[] = []
    upstream.on("connection", (socket) => socket.on("message", (raw) => {
      const message = JSON.parse(raw.toString())
      upstreamRequests.push(message.method)
      socket.send(JSON.stringify({ id: message.id, result: message.method === "thread/start"
        ? { thread: { id: "display-thread" } } : {} }))
    }))
    const requests: string[] = []
    let cancelled = false
    const activeId = origin === "stale" ? "newer-prompt" : "kernel-prompt"
    const session = { agents: [{ id: "agent" }], prompt_states: { agent: { active_prompt: origin === "pending" ? { id: "execution-id", pending_prompt_id: "kernel-prompt" } : { id: activeId }, queued_prompts: [] } } }
    const run = { id: "run" } as import("./cli-types.js").RuntimeProviderRun
    const proxy = await startCodexProxy({
      upstreamEndpoint: `ws://127.0.0.1:${address.port}`,
      client: { send: async (request: Record<string, unknown>) => {
        requests.push(Object.keys(request)[0]!)
        if ("SubmitPrompt" in request) return { PromptSubmitted: { outcome: { Started: { prompt: { id: "kernel-prompt" } } }, session } }
        if ("GetSessionState" in request) return { SessionState: { session: structuredClone(session) } }
        if ("CancelActivePrompt" in request) {
          if (origin === "transition") {
            // MP-08/MP-10: A settled and B started after the snapshot, before dispatch.
            session.prompt_states.agent.active_prompt = { id: "successor-prompt" }
            const expected = (request.CancelActivePrompt as { expected_prompt_id?: string }).expected_prompt_id
            cancelled = expected === undefined || expected === "successor-prompt"
            return { Error: { error: "no active prompt matches the expected identity" } }
          }
          if (origin === "stale") return { Error: { error: "active prompt changed" } }
          assert.deepEqual(request.CancelActivePrompt, { session_id: "room", attachment_id: "attachment", target_agent_id: "agent", expected_prompt_id: "kernel-prompt" })
          cancelled = true
          return { PromptCancelled: { cancellation: {} } }
        }
        throw new Error(`unexpected request ${Object.keys(request)}`)
      } } as unknown as LocalIpcClient,
      sessionId: "room", attachmentId: "attachment", agentId: "agent", model: "model", effort: "medium",
      bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {},
    })
    const proxyAddress = proxy.address()
    assert.ok(proxyAddress && typeof proxyAddress !== "string")
    const native = new WebSocket(`ws://127.0.0.1:${proxyAddress.port}`)
    t.after(async () => {
      native.close()
      await new Promise<void>((resolve) => proxy.close(() => resolve()))
      await new Promise<void>((resolve) => upstream.close(() => resolve()))
    })
    await once(native, "open")
    const notifications: any[] = []
    native.on("message", (raw) => notifications.push(JSON.parse(raw.toString())))
    const response = async (request: { id: number; method: string; params: unknown }) => {
      const received = new Promise<any>((resolve) => {
        const listener = (raw: WebSocket.RawData) => {
          const message = JSON.parse(raw.toString())
          if (message.id === request.id) { native.off("message", listener); resolve(message) }
        }
        native.on("message", listener)
      })
      native.send(JSON.stringify(request))
      return received
    }
    await response({ id: 1, method: "thread/start", params: {} })
    let turnId: string
    if (origin === "native" || origin === "provider") {
      const started = await response({ id: 2, method: "turn/start", params: { threadId: "display-thread", input: [{ type: "text", text: "work" }] } })
      turnId = origin === "provider" ? "provider-executing-turn" : started.result.turn.id
    } else {
      const notification = once(native, "message")
      proxy.projectKernelOutputToTui([{ agent_id: "agent", prompt_id: "kernel-prompt", timestamp_ms: 1000,
        merge_key: "provider-item", source_attachment_id: "attachment", kind: origin === "user" ? "prompt_echo" : "provider_output", bytes: [65] }])
      await notification
      await new Promise<void>((resolve) => setImmediate(resolve))
      turnId = notifications.find((message) => message.method === "turn/started").params.turn.id
    }
    const interrupted = await response({ id: 3, method: "turn/interrupt", params: { threadId: "display-thread", turnId } })
    if (["provider", "stale", "transition"].includes(origin)) assert.ok(interrupted.error)
    else assert.deepEqual(interrupted.result, {})
    assert.equal(cancelled, origin !== "stale" && origin !== "provider" && origin !== "transition")
    assert.equal(upstreamRequests.includes("turn/interrupt"), false)
    assert.equal(requests.includes("GetSessionState"), false)
  })
}
