import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"
import { RelayClientIdentity, createRelayKeypair } from "./relay-crypto.js"

test("a planned reload retries the native event stream promptly while the kernel returns", async () => {
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address(); assert.ok(address && typeof address === "object")
  const attempts: number[] = []
  let availableAt = 0, subscribed = false
  server.on("connection", socket => {
    socket.on("message", data => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "client_connect") {
        if (availableAt) attempts.push(Date.now())
        if (Date.now() < availableAt) {
          socket.send(JSON.stringify({ kind: "close", reason: "target daemon disconnected from relay" }))
          socket.close(); return
        }
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      } else if (frame.kind === "client_subscribe") {
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null, encrypted_response: daemon.encrypt(identity.publicKeyBase64, "null") }))
        if (!subscribed) {
          subscribed = true
          setTimeout(() => {
            availableAt = Date.now() + 1_400
            socket.send(JSON.stringify({ kind: "close", reason: "target daemon disconnected from relay", kernel_reload: true }))
            socket.close()
          }, 20)
        }
      }
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { relayAuthToken: "operator-token", targetDaemonId: "kernel", relayIdentity: identity, kernelPingIntervalMs: 60_000, controlRequestRetryDeadlineMs: 0, reconnectJitterMs: 0 })
  let resumed!: () => void
  const recovery = new Promise<void>(resolve => { resumed = resolve })
  client.onKernelEvent(event => { if (event.event === "transport_resumed") resumed() })
  const timeout = setTimeout(() => resumed(), 5_000)
  try {
    await client.subscribeToKernelEvents("session", "attachment")
    await recovery
    assert.ok(attempts.at(-1)! >= availableAt, "the event stream must actually reconnect")
    const gaps = attempts.slice(1).map((time, index) => time - attempts[index]!)
    assert.ok(gaps.length > 0)
    assert.ok(gaps.every(gap => gap < 600), `planned reload backed off for ${gaps} ms`)
  } finally {
    clearTimeout(timeout)
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})
