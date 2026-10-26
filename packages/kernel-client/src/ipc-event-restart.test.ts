import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { createRelayKeypair } from "./relay-crypto.js"

// MP-08 / MP-10 OWNER-25: the real TUI stayed disconnected after a second
// offline interval because an abandoned subscribe still owned its repair promise.
test("event restart settles the retired carrier's subscribe before binding its replacement", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  let firstSent!: () => void
  const firstSubscribe = new Promise<void>(resolve => { firstSent = resolve })
  let subscribes = 0
  server.on("connection", socket => {
    socket.on("message", raw => {
      const frame = JSON.parse(String(raw))
      if (frame.kind === "client_connect") {
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      }
      if (frame.kind === "client_subscribe") {
        if (++subscribes === 1) { firstSent(); return }
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.client_public_key, "null"), error: null }))
      }
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    relayAuthToken: "supplementary-fixture", targetDaemonId: "kernel", relayIdentity: identity,
  })
  let retiredSettled = false
  const retired = client.subscribeToKernelEvents("room", "attachment").then(
    () => { retiredSettled = true }, () => { retiredSettled = true },
  )
  try {
    await firstSubscribe
    await client.restartKernelEventStream()
    await new Promise(resolve => setTimeout(resolve, 100))
    assert.equal(retiredSettled, true, "a retired carrier cannot hold session repair for the 600-second request timeout")
    await Promise.race([
      client.subscribeToKernelEvents("room", "attachment"),
      new Promise((_, reject) => setTimeout(() => reject(new Error("replacement subscribe stuck")), 1_000)),
    ])
    assert.ok(subscribes >= 2)
  } finally {
    await client.close()
    await retired
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})
