import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"
import { createRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"
import { relayChunkBytes } from "./relay-frame-transport.js"

// MP-08 / MP-10 / MP-11: an encrypted response can take longer than the
// response-stall interval while making continuous, validated transport progress.
test("paced encrypted relay chunks complete without replaying the control request", { timeout: 10_000 }, async t => {
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let connections = 0
  let requests = 0
  const timers = new Set<ReturnType<typeof setTimeout>>()
  const response = { transcript: "x".repeat(512 * 1024) }
  server.on("connection", socket => {
    connections++
    let payload = "", offset = 0
    const next = () => {
      const timer = setTimeout(() => {
        timers.delete(timer)
        if (socket.readyState !== 1 || offset >= payload.length) return
        const part = payload.slice(offset, offset + relayChunkBytes)
        socket.send(JSON.stringify({ kind: "transport_chunk", transfer_id: 1,
          offset, total_bytes: payload.length, payload: part }))
        offset += part.length
      }, 20)
      timers.add(timer)
    }
    socket.on("message", data => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "client_connect") {
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target,
          daemon_public_key: daemon.publicKeyBase64 }))
      } else if (frame.kind === "client_request") {
        requests++
        payload = JSON.stringify({ kind: "client_response", request_id: frame.request_id,
          error: null, encrypted_response: daemon.encrypt(identity.publicKeyBase64, JSON.stringify(response)) })
        next()
      } else if (frame.kind === "transport_ack") {
        next()
      }
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    relayAuthToken: "fixture-token", targetDaemonId: "kernel", relayIdentity: identity,
    controlResponseStallMs: 100, controlRequestRetryDeadlineMs: 2_000,
    kernelPingIntervalMs: 60_000, reconnectJitterMs: 0,
  })
  t.after(async () => {
    client.destroy()
    for (const timer of timers) clearTimeout(timer)
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  })
  const started = Date.now()
  assert.deepEqual(await client.send({ ListSessions: null }), response)
  assert.ok(Date.now() - started > 100, "response takes longer than a stall window")
  assert.equal(connections, 1, "validated progress keeps the admitted socket")
  assert.equal(requests, 1, "the request must not replay during response delivery")
})
