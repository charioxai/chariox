import assert from "node:assert/strict"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { createRelayKeypair } from "./relay-crypto.js"

test("MP-08/MP-11 combined relay refusal cannot reopen retained terminal authority", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address(); assert.ok(address && typeof address === "object")
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  let connections = 0
  server.on("connection", socket => {
    connections++
    socket.on("message", raw => {
      const frame = JSON.parse(String(raw))
      if (frame.kind === "client_connect") socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      if (frame.kind === "client_request") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.encrypted_request.sender_public_key, JSON.stringify({ accepted: true })), error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { relayAuthToken: "synthetic-grant", targetDaemonId: "remote", relayIdentity: identity })
  try {
    client.startRelayAuthRenewal(Date.now()+300_000, async () => ({ token: "synthetic-renewed", expiresAtMs: Date.now()+300_000 }))
    await client.send({ GetDaemonHealth: null })
    for (const socket of server.clients) {
      socket.send(JSON.stringify({ kind: "close", reason: "relay authorization renewal changed identity or reduced permissions" }))
      socket.close()
    }
    await new Promise(resolve => setTimeout(resolve, 30))
    await assert.rejects(client.send({ GetDaemonHealth: null }), (error: unknown) => (error as { code?: string }).code === "authorization_denied")
    assert.equal(connections, 1)
  } finally {
    await client.close(); for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test("active remote terminal crosses repeated grant expiries without reconnecting or resubscribing", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  let connections = 0, subscriptions = 0, events = 0, renewals = 0
  const lifetimes = new Map<WebSocket, ReturnType<typeof setTimeout>>()
  const streams = new Set<ReturnType<typeof setInterval>>()
  server.on("connection", socket => {
    connections++
    socket.on("message", raw => {
      const frame = JSON.parse(String(raw))
      if (frame.kind === "client_connect") {
        clearTimeout(lifetimes.get(socket))
        const expiry = Number(frame.auth_token.split(":")[1])
        lifetimes.set(socket, setTimeout(() => socket.close(), Math.max(0, expiry-Date.now())))
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      }
      if (frame.kind === "client_request") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.encrypted_request.sender_public_key, JSON.stringify({ accepted: true })), error: null }))
      if (frame.kind === "client_subscribe") {
        subscriptions++
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.client_public_key, "null"), error: null }))
        let id = 0
        const stream = setInterval(() => {
          if (socket.readyState !== WebSocket.OPEN) return
          socket.send(JSON.stringify({ kind: "client_event", subscription_id: frame.subscription_id, event_id: ++id, encrypted_event: daemon.encrypt(frame.client_public_key, JSON.stringify({ event: "runtime_notices", notices: [{ message: "continuing" }] })) }))
        }, 25)
        streams.add(stream)
        socket.once("close", () => clearInterval(stream))
      }
    })
  })
  const grant = () => { const expiresAtMs = Date.now()+250; return { token: `fixture:${expiresAtMs}`, expiresAtMs } }
  const first = grant()
  const source = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { relayAuthToken: first.token, targetDaemonId: "home", relayIdentity: identity })
  const release = source.retainForRelayRenewal()
  const target = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { relayAuthToken: first.token, targetDaemonId: "remote", relayIdentity: identity })
  source.startRelayAuthRenewal(first.expiresAtMs, async () => grant())
  target.startRelayAuthRenewal(first.expiresAtMs, async () => {
    // Renewal stays with the issuing kernel after the visible terminal pivots.
    await source.send({ GetDaemonHealth: null })
    renewals++
    return grant()
  }, release)
  target.onKernelEvent(event => { if (event.event === "runtime_notices") events++ })
  try {
    await source.send({ GetDaemonHealth: null })
    await target.subscribeToKernelEvents("session-fixture", "attachment-fixture")
    await source.close() // Retained authority survives the visible client closing.
    const deadline = Date.now()+1_500
    while (Date.now()<deadline) {
      assert.deepEqual(await target.send({ GetDaemonHealth: null }), { accepted: true })
      await new Promise(resolve => setTimeout(resolve, 30))
    }
    assert.ok(renewals>=6, "must span several original grant lifetimes")
    assert.ok(events>20, "the terminal stream must keep delivering events")
    assert.equal(subscriptions, 1)
    assert.equal(connections, 3) // One retained issuer/control and target/control+event.
  } finally {
    await target.close()
    for (const timer of lifetimes.values()) clearTimeout(timer)
    for (const stream of streams) clearInterval(stream)
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test("background renewal cannot block active commands and revocation cannot reopen old authority", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  let connections = 0
  server.on("connection", socket => {
    connections++
    socket.on("message", raw => {
      const frame = JSON.parse(String(raw))
      if (frame.kind === "client_connect") socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      if (frame.kind === "client_request") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.encrypted_request.sender_public_key, JSON.stringify({ accepted: true })), error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { relayAuthToken: "synthetic-valid-grant", targetDaemonId: "remote", relayIdentity: identity })
  const notices: string[] = []
  client.onKernelEvent(event => { if (event.event === "transport_closed") notices.push(event.message) })
  let release!: () => void, renewing!: () => void
  const pending = new Promise<void>(resolve => { release = resolve })
  const started = new Promise<void>(resolve => { renewing = resolve })
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    await client.send({ GetDaemonHealth: null })
    client.startRelayAuthRenewal(Date.now()+80, async () => {
      renewing()
      await pending
      throw Object.assign(new Error("Client authority revoked"), { code: "client_revoked" })
    })
    await started
    const result = await Promise.race([
      client.send({ GetDaemonHealth: null }),
      new Promise<never>((_, reject) => { deadline = setTimeout(() => reject(new Error("renewal blocked an active command")), 300) }),
    ])
    assert.deepEqual(result, { accepted: true })
    release()
    await new Promise(resolve => setTimeout(resolve, 25))
    await assert.rejects(client.send({ GetDaemonHealth: null }), (error: unknown) => (error as {code?: string}).code === "client_revoked")
    assert.equal(connections, 1)
    assert.equal(notices.length, 1, "background revocation must retire the visible session")
    client.invalidateRelayAuthorization(Object.assign(new Error("revoked again"), { code: "client_revoked" }))
    assert.equal(notices.length, 1, "revocation must notify the UI exactly once")
  } finally {
    release()
    clearTimeout(deadline)
    await client.close()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})


test("MP-08/MP-10/MP-11 explicit client-family invalidation closes the visible session once", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address(); assert.ok(address && typeof address === "object")
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  let connections = 0
  server.on("connection", socket => {
    connections++
    socket.on("message", raw => {
      const frame = JSON.parse(String(raw))
      if (frame.kind === "client_connect") socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      if (frame.kind === "client_request") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.encrypted_request.sender_public_key, JSON.stringify({ accepted: true })), error: null }))
      if (frame.kind === "client_subscribe") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.client_public_key, "null"), error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { relayAuthToken: "synthetic-valid-grant", targetDaemonId: "remote", relayIdentity: identity })
  const notices: string[] = []
  client.onKernelEvent(event => { if (event.event === "transport_closed") notices.push(event.message) })
  try {
    client.startRelayAuthRenewal(Date.now()+300_000, async () => ({ token: "synthetic-renewed", expiresAtMs: Date.now()+300_000 }))
    await client.send({ GetDaemonHealth: null })
    await client.subscribeToKernelEvents("session-fixture", "attachment-fixture")
    const error = Object.assign(new Error("Client authority revoked"), { code: "client_revoked" })
    client.invalidateRelayAuthorization(error)
    client.invalidateRelayAuthorization(error)
    assert.equal(notices.length, 1, "revocation must retire both lanes and notify the visible session exactly once")
    await assert.rejects(client.send({ GetDaemonHealth: null }), (failure: unknown) => (failure as { code?: string }).code === "client_revoked")
    assert.equal(connections, 2, "revoked authority must not reconnect either lane")
  } finally {
    await client.close()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})
