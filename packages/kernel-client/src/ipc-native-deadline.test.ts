import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, LocalIpcError } from "./ipc.js"

test("MP-08 / MP-10 / MP-11 scoped cancellation retires a silent native IPC wait without closing the shared socket", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`)
  const controller = new AbortController()
  let requests = 0
  let lateReply: (() => void) | undefined
  let written!: () => void
  const firstWritten = new Promise<void>(resolve => { written = resolve })
  server.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data))
    requests += 1
    const reply = () => socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
    if (requests === 1) { lateReply = reply; written() } else reply()
  }))
  try {
    const pending = client.send({ NativeProviderTransport: {
      session_id: "session", attachment_id: "attachment", agent_id: "agent", provider_run_id: "run",
      operation: { kind: "exchange", connection_id: "connection", frames: [], sequence: 1 },
    } }, { signal: controller.signal })
    const rejected = assert.rejects(pending, (error: unknown) => error instanceof LocalIpcError && error.code === "outcome_unknown")
    await firstWritten
    const before = Date.now()
    controller.abort(new LocalIpcError("native recovery", "attempt expired", "outcome_unknown", true))
    await rejected
    assert.ok(Date.now() - before < 250)
    // A late response cannot resurrect the retired wait or consume the next
    // request. The shared socket remains usable by other kernel operations.
    lateReply?.()
    assert.deepEqual(await client.send({ ListSessions: null }), { ok: true })
    assert.equal(requests, 2)
    assert.equal(server.clients.size, 1)
  } finally {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})
