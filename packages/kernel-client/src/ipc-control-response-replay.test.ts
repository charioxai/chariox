import assert from "node:assert/strict"
import test from "node:test"

import WebSocket, { WebSocketServer } from "ws"

import { LocalIpcClient, LocalIpcError } from "./ipc.js"

test("LocalIpcClient reconnects and replays a command when its response stalls", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))

  const address = server.address()
  assert.ok(address && typeof address === "object")
  const received: Array<{ request_id: string; command_id: string }> = []
  server.on("connection", (socket) => {
    socket.once("message", (payload) => {
      const frame = JSON.parse(String(payload)) as {
        request_id: string
        command_id: string
      }
      received.push(frame)
      if (received.length === 1) {
        return
      }
      socket.send(JSON.stringify({
        type: "response",
        request_id: frame.request_id,
        response: { ok: true },
        error: null,
      }))
    })
  })

  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 2_000,
    controlResponseStallMs: 25,
    reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) {
      socket.terminate()
    }
    return new Promise<void>((resolve) => server.close(() => resolve()))
  })

  const response = await client.send<{ ok: boolean }>({ ListSessions: null })

  assert.deepEqual(response, { ok: true })
  assert.equal(received.length, 2)
  assert.equal(received[0]?.request_id, received[1]?.request_id)
  assert.equal(received[0]?.command_id, received[1]?.command_id)
})

for (const request of [
  { SubmitPrompt: { session_id: "fixture", prompt: "  /sudo protected task" } },
  { RequestKernelAccess: { holder_pid: process.pid } },
  { RequestKernelSudo: { agent_id: "fixture", prompt: "external task" } },
]) {
  const kind = Object.keys(request)[0]
  test(`LocalIpcClient waits past control stalls for ${kind} without replay`, async (t) => {
    const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
    await new Promise<void>((resolve) => server.once("listening", resolve))
    const address = server.address()
    assert.ok(address && typeof address === "object")
    let received = 0
    server.on("connection", (socket) => {
      socket.on("message", (payload) => {
        const frame = JSON.parse(String(payload))
        received++
        setTimeout(() => socket.send(JSON.stringify({
          type: "response", request_id: frame.request_id,
          response: { ok: true }, error: null,
        })), 100)
      })
    })
    const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
      controlRequestRetryDeadlineMs: 2_000, controlResponseStallMs: 10,
      reconnectJitterMs: 0,
    })
    t.after(() => {
      client.destroy()
      for (const socket of server.clients) socket.terminate()
      return new Promise<void>((resolve) => server.close(() => resolve()))
    })
    assert.deepEqual(await client.send(request), { ok: true })
    assert.equal(received, 1)
  })

  test(`LocalIpcClient fails a disconnected ${kind} without replay`, async (t) => {
    const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
    await new Promise<void>((resolve) => server.once("listening", resolve))
    const address = server.address()
    assert.ok(address && typeof address === "object")
    let received = 0
    server.on("connection", (socket) => {
      socket.once("message", () => { received++; socket.close() })
    })
    const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
      controlRequestRetryDeadlineMs: 2_000, reconnectJitterMs: 0,
    })
    t.after(() => {
      client.destroy()
      for (const socket of server.clients) socket.terminate()
      return new Promise<void>((resolve) => server.close(() => resolve()))
    })
    await assert.rejects(client.send(request), /closed/)
    assert.equal(received, 1)
  })

  test(`a stalled sibling fails an untimed ${kind} while replaying the sibling`, { timeout: 3_000 }, async (t) => {
    const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
    await new Promise<void>((resolve) => server.once("listening", resolve))
    const address = server.address()
    assert.ok(address && typeof address === "object")
    let authorizationReceived!: () => void
    const ready = new Promise<void>(resolve => { authorizationReceived = resolve })
    let authorizationCount = 0
    const siblings: string[] = []
    server.on("connection", socket => {
      socket.on("message", payload => {
        const frame = JSON.parse(String(payload))
        if (!("ListSessions" in frame.request)) {
          authorizationCount++
          authorizationReceived()
          return
        }
        siblings.push(frame.request_id)
        if (siblings.length === 1) return
        socket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
          response: { ok: true }, error: null }))
      })
    })
    const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
      controlRequestRetryDeadlineMs: 2_000, controlResponseStallMs: 25, reconnectJitterMs: 0,
    })
    t.after(() => {
      client.destroy()
      for (const socket of server.clients) socket.terminate()
      return new Promise<void>(resolve => server.close(() => resolve()))
    })
    // Reflect errors immediately, including rejection during the sibling retry.
    const authorization = client.send(request).then(() => null, error => error)
    await ready
    assert.deepEqual(await client.send({ ListSessions: null }), { ok: true })
    const error = await authorization
    assert.ok(error instanceof LocalIpcError)
    assert.equal(error.code, "connection_closed")
    assert.equal(error.retryable, true)
    assert.equal(authorizationCount, 1)
    assert.equal(siblings.length, 2)
    assert.equal(siblings[0], siblings[1])
  })

  for (const failure of ["missed pong", "ping failed"] as const) {
    test(`a ${failure} heartbeat fails an untimed ${kind} without replay`, { timeout: 3_000 }, async (t) => {
      if (failure === "ping failed") {
        t.mock.method(WebSocket.prototype, "ping", () => { throw new Error("fixture ping failure") })
      }
      const server = new WebSocketServer({ host: "127.0.0.1", port: 0, autoPong: false })
      await new Promise<void>((resolve) => server.once("listening", resolve))
      const address = server.address()
      assert.ok(address && typeof address === "object")
      let authorizationCount = 0
      server.on("connection", socket => {
        socket.on("message", payload => {
          const frame = JSON.parse(String(payload))
          if (!("ListSessions" in frame.request)) { authorizationCount++; return }
          socket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
            response: { ok: true }, error: null }))
        })
      })
      const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
        kernelPingIntervalMs: 250, kernelMaxMissedPongs: 1, controlRequestRetryDeadlineMs: 2_000,
      })
      t.after(() => {
        client.destroy()
        for (const socket of server.clients) socket.terminate()
        return new Promise<void>(resolve => server.close(() => resolve()))
      })
      await assert.rejects(client.send(request), (error: unknown) => {
        assert.ok(error instanceof LocalIpcError)
        assert.equal(error.code, "connection_closed")
        assert.equal(error.retryable, true)
        return true
      })
      assert.equal(authorizationCount, 1)
      assert.deepEqual(await client.send({ ListSessions: null }), { ok: true })
    })
  }

}
