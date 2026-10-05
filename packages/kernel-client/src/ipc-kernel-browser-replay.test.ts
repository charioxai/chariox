import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, LocalIpcError } from "./ipc.js"

for (const op of ["input", "open", "navigate", "close", "subscribe", "stop"]) {
  test(`MD-3: disconnected browser ${op} is never replayed as a new terminal actor`, async t => {
    const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
    await new Promise<void>(resolve => server.once("listening", resolve))
    const address = server.address()
    assert.ok(address && typeof address === "object")
    let requests = 0
    server.on("connection", socket => socket.on("message", payload => {
      requests++
      if (requests === 1) socket.terminate()
      else socket.send(JSON.stringify({ type: "response", request_id: JSON.parse(String(payload)).request_id,
        response: { duplicatedMutation: true }, error: null }))
    }))
    const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
      controlRequestRetryDeadlineMs: 1_000, controlResponseStallMs: 10, reconnectJitterMs: 0,
    })
    t.after(() => {
      client.destroy()
      for (const socket of server.clients) socket.terminate()
      return new Promise<void>(resolve => server.close(() => resolve()))
    })
    await assert.rejects(client.send({ KernelBrowser: { command: { op } } }),
      (error: unknown) => error instanceof LocalIpcError && error.code === "outcome_unknown" && !error.retryable)
    assert.equal(requests, 1, "MD-3: input/open must not execute twice")
  })
}
