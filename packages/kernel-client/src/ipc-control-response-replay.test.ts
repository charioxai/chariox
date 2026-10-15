import assert from "node:assert/strict"
import test from "node:test"

import WebSocket, { WebSocketServer } from "ws"

import { LocalIpcClient, LocalIpcError } from "./ipc.js"
import { createRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"

test("MP-08/MP-10/MP-11 a delayed encrypted relay response settles a same-socket replay without a paired identity", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  let connections = 0
  let pings = 0
  let attempts = 0
  let executions = 0
  const commands = new Set<string>()
  let firstReply: string | null = null
  server.on("connection", socket => {
    connections++
    socket.on("ping", () => { pings++ })
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      if (frame.kind === "client_connect") {
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
        return
      }
      assert.equal(frame.kind, "client_request")
      attempts++
      const command = JSON.parse(daemon.decrypt(frame.encrypted_request))
      if (!commands.has(command.command_id)) { commands.add(command.command_id); executions++ }
      const response = JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null,
        encrypted_response: daemon.encrypt(frame.encrypted_request.sender_public_key, JSON.stringify({ ok: true })) })
      if (attempts === 1) { firstReply = response; return }
      assert.ok(firstReply)
      socket.send(firstReply)
      socket.send(response)
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    relayAuthToken: "fixture-token", targetDaemonId: "daemon-1",
    kernelPingIntervalMs: 250, kernelMaxMissedPongs: 2,
    controlRequestRetryDeadlineMs: 3_000, controlResponseStallMs: 350, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await client.send({ CreateSession: { name: "slow mutation" } }), { ok: true })
  assert.equal(attempts, 2, "the first encrypted reply must arrive after its replay starts")
  assert.equal(executions, 1, "the replay retains command deduplication")
  assert(pings > 0)
  assert.equal(connections, 1)
})

test("MP-08/MP-10 slow responses do not retire a control socket still answering heartbeats", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let connections = 0
  let pings = 0
  const commands = new Map<string, Promise<{ ok: boolean }>>()
  server.on("connection", (socket) => {
    connections++
    socket.on("ping", () => { pings++ })
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload))
      let result = commands.get(frame.request_id)
      if (!result) {
        result = new Promise(resolve => setTimeout(() => resolve({ ok: true }), 600))
        commands.set(frame.request_id, result)
      }
      void result.then(response => {
        if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({
          type: "response", request_id: frame.request_id, response, error: null,
        }))
      })
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    kernelPingIntervalMs: 250, kernelMaxMissedPongs: 2,
    controlRequestRetryDeadlineMs: 3_000, controlResponseStallMs: 350,
    reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await Promise.all([
    client.send({ ListSessions: null }), client.send({ GetKernelCapabilities: null }),
  ]), [{ ok: true }, { ok: true }])
  assert.equal(commands.size, 2, "MP-10 replay must preserve each command identity")
  assert(pings > 0, "MP-10 the real WebSocket must answer a heartbeat before the response stall")
  assert.equal(connections, 1, "MP-10 a response stall must not retire the responsive serving carrier")
})

test("MP-08 LocalIpcClient replays a stalled command and records the retired lane", async (t) => {
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

  const diagnostics: Array<{ cause: string; lane: string }> = []
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    onTransportDiagnostic: (event) => diagnostics.push(event),
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
  assert(diagnostics.some(event => event.cause === "request_replay" && event.lane === "control"), "MP-08 replay must expose the serving lane retirement")
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
