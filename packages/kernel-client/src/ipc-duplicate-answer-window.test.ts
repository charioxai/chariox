import assert from "node:assert/strict"
import test from "node:test"

import { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"

test("a command that ran past the retry deadline is answered once the kernel's retry-answer window opens", { timeout: 15_000 }, async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let attempts = 0
  server.on("connection", socket => {
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      attempts++
      // The first attempt stalls like a slow slice save; the kernel then reports the finished
      // result as unavailable until its answer window for the replays opens on the fourth attempt.
      if (attempts === 1) return
      socket.send(JSON.stringify(attempts < 4
        ? { type: "response", request_id: frame.request_id, response: null,
          error: { code: "duplicate_command_unavailable", message: "original duplicate command result was unavailable", retryable: true } }
        : { type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 250, controlResponseStallMs: 100, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await client.send({ CreateSession: { name: "long mutation" } }), { ok: true })
  assert.equal(attempts, 4)
})
