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

test("a kernel that keeps refusing a finished command ends the retries once, one extension after the first refusal", { timeout: 15_000 }, async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let attempts = 0
  server.on("connection", socket => {
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      attempts++
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: null,
        error: { code: "duplicate_command_unavailable", message: "original duplicate command result was unavailable", retryable: true } }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 200, duplicateAnswerRetryMs: 600, controlResponseStallMs: 100, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  const startedAt = Date.now()
  await assert.rejects(client.send({ CreateSession: { name: "long mutation" } }), { code: "duplicate_command_unavailable" })
  const elapsedMs = Date.now() - startedAt
  assert.ok(elapsedMs >= 600 && elapsedMs < 2_000, `retries ended after ${elapsedMs} ms, expected one 600 ms extension from the first refusal`)
  assert.ok(attempts > 1)
})

test("a first refusal that arrives after the original deadline and its extension still recovers the answer", { timeout: 15_000 }, async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let refusalSent = false
  let latest: { socket: import("ws").WebSocket, requestId: string } | null = null
  // A slow slice save: the kernel answers nothing until 900 ms, past the 200 ms deadline plus the
  // 300 ms extension, then refuses the pending replay while its answer throttle is closed.
  setTimeout(() => {
    assert.ok(latest)
    refusalSent = true
    latest.socket.send(JSON.stringify({ type: "response", request_id: latest.requestId, response: null,
      error: { code: "duplicate_command_unavailable", message: "original duplicate command result was unavailable", retryable: true } }))
  }, 900)
  server.on("connection", socket => {
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      if (!refusalSent) {
        latest = { socket, requestId: frame.request_id }
        return
      }
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 200, duplicateAnswerRetryMs: 300, controlResponseStallMs: 100, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await client.send({ CreateSession: { name: "long mutation" } }), { ok: true })
})
