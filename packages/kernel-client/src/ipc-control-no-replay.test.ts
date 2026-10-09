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
  { KernelBrowser: { command: { op: "mirror_next", subscription_id: "s", generation: 1, after_sequence: 0, drift_nodes: [] } } },
  { KernelBrowser: { command: { op: "display_next", subscription_id: "s", generation: 1, after_sequence: 2 } } },
  { OpenUserAppView: { installation_id: "todo", host: "client_native" } },
  { CallUserAppView: { view_id: "user-app-fixture", method: "increment", input: {} } },
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
      error instanceof LocalIpcError && error.code === "outcome_unknown" && !error.retryable)
    await new Promise((resolve) => setTimeout(resolve, 300))
    assert.equal(state.requests, 1)
  })
}

test("LocalIpcClient ends an App worker control at once when the heartbeat drops its socket", async (t) => {
  // The server never answers and never pongs: a half-open link.
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0, autoPong: false })
  await new Promise<void>((resolve) => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let requests = 0
  server.on("connection", (socket) => socket.on("message", () => { requests += 1 }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlResponseStallMs: 25, kernelPingIntervalMs: 250, kernelMaxMissedPongs: 1, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>((resolve) => server.close(() => resolve()))
  })

  const started = Date.now()
  await assert.rejects(client.send({ ControlAppWorker: { installation_id: "todo", action: "restart" } }),
    (error: unknown) => error instanceof LocalIpcError && error.code === "outcome_unknown")
  assert.ok(Date.now() - started < 5_000, "not left waiting for the request timeout")
  assert.equal(requests, 1)
})

test("LocalIpcClient ends an App worker control at once when another request's replay drops its socket", async (t) => {
  // Neither first request is answered; the read stalls, drops the socket and is answered on replay.
  const { client, state } = await kernel(t, (socket, frame, count) => {
    if (count <= 2) return
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
  })
  const control = client.send({ ControlAppWorker: { installation_id: "todo", action: "restart" } })
  const outcome = control.then(() => null, (error: unknown) => error)
  await new Promise((resolve) => setTimeout(resolve, 20))
  assert.deepEqual(await client.send({ GetAppWorker: { installation_id: "todo" } }), { ok: true })
  const error = await outcome
  assert.ok(error instanceof LocalIpcError && error.code === "outcome_unknown")
  assert.equal(state.requests, 3)
})

test("LocalIpcClient keeps the transport error of an App worker control it never wrote", async () => {
  // Nothing listens: the request is never written, so its outcome is known.
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  await new Promise<void>((resolve) => server.close(() => resolve()))
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 300,
    reconnectJitterMs: 0,
  })
  try {
    await assert.rejects(client.send({ ControlAppWorker: { installation_id: "todo", action: "restart" } }),
      (error: unknown) => error instanceof LocalIpcError && error.code !== "outcome_unknown")
  } finally {
    client.destroy()
  }
})

test("LocalIpcClient still replays a stalled read", async (t) => {
  const { client, state } = await kernel(t, (socket, frame, count) => {
    if (count === 1) return
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
  })

  assert.deepEqual(await client.send({ GetAppWorker: { installation_id: "todo" } }), { ok: true })
  assert.equal(state.requests, 2)
})
