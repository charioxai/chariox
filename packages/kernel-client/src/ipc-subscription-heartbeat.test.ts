import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { createRelayKeypair } from "./relay-crypto.js"

// MP-08 / MP-10 / MP-11: public methods must apply repeat options on both transports.
for (const relay of [false, true]) {
  for (const scope of ["session", "waiting_room_inventory"] as const) {
    test(`${relay ? "relay" : "direct"} ${scope} replaces heartbeat options`, async t => {
      const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
      await new Promise<void>(resolve => server.once("listening", resolve))
      const address = server.address()
      assert.ok(address && typeof address === "object")
      const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
      const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
      const intervals: Array<number | undefined> = []
      const live = new Set<string>()
      server.on("connection", socket => socket.on("message", raw => {
        const frame = JSON.parse(String(raw))
        if (frame.kind === "client_connect") {
          socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
          return
        }
        if (relay) {
          if (frame.kind === "client_subscribe") {
            intervals.push(frame.heartbeat_interval_ms)
            live.add(frame.subscription_id)
          } else if (frame.kind === "client_unsubscribe") {
            live.delete(frame.subscription_id)
          } else return
          socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(identity.publicKeyBase64, "null"), error: null }))
        } else {
          const request = frame
          if (request?.type === "subscribe") intervals.push(request.heartbeat_interval_ms)
          socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
        }
      }))
      const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, relay ? {
        relayAuthToken: "supplementary-fixture", targetDaemonId: "kernel", relayIdentity: identity,
      } : {})
      t.after(async () => {
        await client.close()
        for (const socket of server.clients) socket.terminate()
        await new Promise<void>(resolve => server.close(() => resolve()))
      })
      const subscribe = (heartbeatIntervalMs?: number) => scope === "session"
        ? client.subscribeToKernelEvents("room", "attachment", heartbeatIntervalMs === undefined ? {} : { heartbeatIntervalMs })
        : client.subscribeToWaitingRoomInventory(heartbeatIntervalMs === undefined ? {} : { heartbeatIntervalMs })
      await subscribe()
      await subscribe(500)
      await subscribe(1_000)
      await subscribe()
      assert.deepEqual(intervals, [undefined, 500, 1_000, undefined])
      if (relay) assert.equal(live.size, 1, "a cadence change retires the old kernel stream")
      await subscribe()
      if (relay) assert.equal(intervals.length, 4, "unchanged relay options retain their binding")
    })
  }
}
