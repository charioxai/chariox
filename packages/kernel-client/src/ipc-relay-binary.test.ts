import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"
import { createRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"
import { relayBinaryPayloadBytes } from "./relay-frame-transport.js"
import type { KernelEvent } from "./kernel-events.js"

test("native relay negotiates binary and decrypts a chunked raw terminal reply", { timeout: 10_000 }, async t => {
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const source = Buffer.from("\u001b[32mterminal bytes\u001b[0m\n".repeat(4096))
  const header = Buffer.from(JSON.stringify({ TerminalOutput: { records: [{ bytes: [0, source.length] }] } }))
  const terminal = Buffer.concat([Buffer.from("CXT1"), Buffer.alloc(4), header, source])
  terminal.writeUInt32BE(header.length, 4)
  const protocols: string[] = []
  let receipts = 0
  let expectedReceipts = 0
  let finishAcknowledging!: () => void
  const acknowledged = new Promise<void>(resolve => { finishAcknowledging = resolve })
  server.on("connection", socket => {
    protocols.push(socket.protocol)
    socket.on("message", data => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "client_connect") {
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      } else if (frame.kind === "client_request") {
        const encrypted = daemon.encrypt(identity.publicKeyBase64, terminal)
        const routing = Buffer.from(JSON.stringify({ kind: "client_response", request_id: frame.request_id,
          sender_public_key: encrypted.sender_public_key, nonce: encrypted.nonce }))
        const wire = Buffer.concat([Buffer.from("CXR2"), Buffer.alloc(4), routing, Buffer.from(encrypted.ciphertext, "base64")])
        wire.writeUInt32BE(routing.length, 4)
        expectedReceipts = Math.ceil(wire.length / relayBinaryPayloadBytes)
        for (let offset = 0; offset < wire.length; offset += relayBinaryPayloadBytes) {
          const payload = wire.subarray(offset, offset + relayBinaryPayloadBytes)
          const chunk = Buffer.concat([Buffer.from("CXB1"), Buffer.alloc(12), payload])
          chunk.writeUInt32BE(1, 4); chunk.writeUInt32BE(offset, 8); chunk.writeUInt32BE(wire.length, 12)
          socket.send(chunk)
        }
      } else if (frame.kind === "transport_ack" && ++receipts === expectedReceipts) finishAcknowledging()
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    relayAuthToken: "fixture-token", targetDaemonId: "kernel", relayIdentity: identity,
    controlResponseStallMs: 100, controlRequestRetryDeadlineMs: 200,
    kernelPingIntervalMs: 60_000, reconnectJitterMs: 0,
  })
  t.after(async () => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await client.send({ ListSessions: null }), { TerminalOutput: { records: [{ bytes: Array.from(source) }] } })
  assert.deepEqual(protocols, ["chariox-relay-binary-v98"])
  await acknowledged
  assert.ok(receipts > 1, "binary delivery acknowledges its contiguous chunks")
})

test("native relay decrypts a binary raw terminal event on its subscription", { timeout: 10_000 }, async t => {
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const source = Buffer.from([0, 255, 27, 91, 51, 50, 109])
  const metadata = Buffer.from(JSON.stringify({ event: "terminal_output", records: [{ bytes: [0, source.length] }] }))
  const terminal = Buffer.concat([Buffer.from("CXT1"), Buffer.alloc(4), metadata, source])
  terminal.writeUInt32BE(metadata.length, 4)
  server.on("connection", socket => {
    socket.on("message", data => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "client_connect") {
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      } else if (frame.kind === "client_subscribe") {
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null,
          encrypted_response: daemon.encrypt(identity.publicKeyBase64, "null") }))
        const encrypted = daemon.encrypt(identity.publicKeyBase64, terminal)
        const routing = Buffer.from(JSON.stringify({ kind: "client_event", subscription_id: frame.subscription_id,
          event_id: 1, sender_public_key: encrypted.sender_public_key, nonce: encrypted.nonce }))
        const wire = Buffer.concat([Buffer.from("CXR1"), Buffer.alloc(4), routing, Buffer.from(encrypted.ciphertext, "base64")])
        wire.writeUInt32BE(routing.length, 4)
        socket.send(wire)
      }
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    relayAuthToken: "fixture-token", targetDaemonId: "kernel", relayIdentity: identity,
    kernelPingIntervalMs: 60_000, reconnectJitterMs: 0,
  })
  t.after(async () => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  })
  const event = new Promise<KernelEvent>(resolve => client.onKernelEvent(value => {
    if (value.event === "terminal_output") resolve(value)
  }))
  await client.subscribeToKernelEvents("session", "attachment")
  assert.deepEqual(await event, { event: "terminal_output", records: [{ bytes: Array.from(source) }] })
})
