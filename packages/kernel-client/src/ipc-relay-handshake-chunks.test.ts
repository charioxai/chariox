import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"
import { createRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"

// MP-08 / MP-10 / MP-11: a relay that accepted chunks-v2 frames every reply,
// including the connect acknowledgement, as a transport chunk.
test("relay handshake accepts a chunk-framed client_connected", { timeout: 10_000 }, async t => {
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let transfer = 0
  const chunked = (frame: object) => {
    const payload = JSON.stringify(frame)
    return JSON.stringify({ kind: "transport_chunk", transfer_id: ++transfer, offset: 0, total_bytes: payload.length, payload })
  }
  const response = { sessions: [] }
  server.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data))
    if (frame.kind === "client_connect") {
      socket.send(chunked({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
    } else if (frame.kind === "client_request") {
      socket.send(chunked({ kind: "client_response", request_id: frame.request_id, error: null,
        encrypted_response: daemon.encrypt(identity.publicKeyBase64, JSON.stringify(response)) }))
    }
  }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    relayAuthToken: "fixture-token", targetDaemonId: "kernel", relayIdentity: identity,
    kernelPingIntervalMs: 60_000, reconnectJitterMs: 0,
  })
  t.after(async () => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await client.send({ ListSessions: null }), response)
})
