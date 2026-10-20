import assert from "node:assert/strict"
import test from "node:test"

import { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"

// MP-08 / MP-10 PANE-03: "kernel requests on open" is measured from the client,
// by request kind only (never payloads).
test("LocalIpcClient counts the control requests it writes by kind", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  server.on("connection", (socket) => socket.on("message", (payload) => {
    const frame = JSON.parse(String(payload)) as { request_id: string }
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
  }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`)
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>((resolve) => server.close(() => resolve()))
  })
  assert.deepEqual(client.requestStats(), { count: 0, kinds: {}, timeline: [] })
  await client.send({ ListSessions: null })
  await client.send({ ListSessions: null })
  await client.send({ GetSessionState: { session_id: "secret-session" } })
  const stats = client.requestStats()
  assert.equal(stats.kinds.ListSessions, 2)
  assert.equal(stats.kinds.GetSessionState, 1)
  assert.equal(stats.count, Object.values(stats.kinds).reduce((sum, n) => sum + n, 0))
  assert.ok(stats.count >= 3)
  assert.deepEqual(stats.timeline.slice(-3).map(([kind]) => kind), ["ListSessions", "ListSessions", "GetSessionState"])
  assert.equal(JSON.stringify(stats).includes("secret-session"), false)
})
