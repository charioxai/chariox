import assert from "node:assert/strict"
import test from "node:test"

import { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"

test("LocalIpcClient reconnects and replays a command when its response stalls", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))

  const address = server.address()
  assert.ok(address && typeof address === "object")
  const received: Array<{ request_id: string; command_id: string }> = []
  server.on("connection", (socket) => {
    socket.once("message", (payload) => {
      const frame = JSON.parse(String(payload)) as {
        request_id: string
        command_id: string
      }
      received.push(frame)
      if (received.length === 1) {
        return
      }
      socket.send(JSON.stringify({
        type: "response",
        request_id: frame.request_id,
        response: { ok: true },
        error: null,
      }))
    })
  })

  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 2_000,
    controlResponseStallMs: 25,
    reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) {
      socket.terminate()
    }
    return new Promise<void>((resolve) => server.close(() => resolve()))
  })

  const response = await client.send<{ ok: boolean }>({ ListSessions: null })

  assert.deepEqual(response, { ok: true })
  assert.equal(received.length, 2)
  assert.equal(received[0]?.request_id, received[1]?.request_id)
  assert.equal(received[0]?.command_id, received[1]?.command_id)
})
