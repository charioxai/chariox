import assert from "node:assert/strict"
import test, { type TestContext } from "node:test"

import { WebSocketServer, type WebSocket } from "ws"

import { LocalIpcClient, LocalIpcError } from "./ipc.js"

// A worker control or an uninstall runs again when replayed (no request-id
// deduplication in the kernel), so a slow one must be waited for, not resent.
async function kernel(t: TestContext, onRequest: (socket: WebSocket, frame: { request_id: string }, count: number) => void) {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const state = { connections: 0, requests: 0 }
  server.on("connection", (socket) => {
    state.connections += 1
    socket.on("message", (payload) => {
      state.requests += 1
      onRequest(socket, JSON.parse(String(payload)) as { request_id: string }, state.requests)
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 2_000,
    controlResponseStallMs: 25,
    reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>((resolve) => server.close(() => resolve()))
  })
  return { client, state }
}

for (const request of [
  { ControlAppWorker: { installation_id: "todo", action: "restart" } },
  { UninstallApp: { installation_id: "todo", expected_generation: "3" } },
]) {
  const kind = Object.keys(request)[0]

  test(`LocalIpcClient waits for a slow ${kind} instead of replaying it`, async (t) => {
    const { client, state } = await kernel(t, (socket, frame) => {
      // Answer well past the 25 ms stall window, as a slow worker stop does.
      setTimeout(() => socket.send(JSON.stringify({
        type: "response", request_id: frame.request_id, response: { ok: true }, error: null,
      })), 300)
    })

    assert.deepEqual(await client.send(request), { ok: true })
    assert.equal(state.requests, 1)
    assert.equal(state.connections, 1)
  })

  test(`LocalIpcClient does not resend a ${kind} whose connection closed after it was sent`, async (t) => {
    const { client, state } = await kernel(t, (socket) => socket.terminate())

    await assert.rejects(client.send(request), (error: unknown) =>
      error instanceof LocalIpcError && error.code === "connection_closed")
    await new Promise((resolve) => setTimeout(resolve, 300))
    assert.equal(state.requests, 1)
  })
}

test("LocalIpcClient still replays a stalled read", async (t) => {
  const { client, state } = await kernel(t, (socket, frame, count) => {
    if (count === 1) return
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
  })

  assert.deepEqual(await client.send({ GetAppWorker: { installation_id: "todo" } }), { ok: true })
  assert.equal(state.requests, 2)
})
