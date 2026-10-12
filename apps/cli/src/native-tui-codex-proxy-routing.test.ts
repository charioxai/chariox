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
