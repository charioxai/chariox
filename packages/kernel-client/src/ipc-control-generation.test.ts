import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"

test("control capability and mutation use the same live WebSocket", async t => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let connections = 0
  server.on("connection", socket => {
    connections++
    let admitted = false
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      let response: unknown
      if (frame.request.RelayStatus === null) {
        admitted = true
        response = { RelayStatus: { status: { daemon_id: "home-1", machine_id: "machine-1",
          capabilities: ["disposable_worker_control_v1"] } } }
      } else response = { admitted }
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response, error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`)
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  assert.deepEqual(await client.send({ ReleaseDisposableWorker: { homeKernelId: "home-1", allocationId: "allocation-1" } }), { admitted: true })
  assert.equal(connections, 1)
})

test("control mutation is not replayed onto an unadmitted replacement connection", async t => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let connections = 0
  const mutations: number[] = []
  server.on("connection", socket => {
    const generation = ++connections
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      let response: unknown
      if (frame.request.RelayStatus === null) {
        response = { RelayStatus: { status: { daemon_id: "home-1", machine_id: "machine-1",
          capabilities: generation === 1 ? ["disposable_worker_control_v1"] : [] } } }
      } else {
        mutations.push(generation)
        if (generation === 1) { socket.terminate(); return }
        response = { unexpectedlyMutated: true }
      }
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response, error: null }))
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    controlRequestRetryDeadlineMs: 2_000, controlResponseStallMs: 25, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  await assert.rejects(client.send({ ReleaseDisposableWorker: { homeKernelId: "home-1", allocationId: "allocation-1" } }))
  assert.deepEqual(mutations, [1])
})
