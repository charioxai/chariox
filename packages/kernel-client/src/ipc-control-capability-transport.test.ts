import assert from "node:assert/strict"
import test from "node:test"
import { createServer } from "node:http"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"

test("bare Unix controls use websocket frames and fresh capabilities", async t => {
  const scratch = await mkdtemp(join(tmpdir(), "cx-control-"))
  const endpoint = join(scratch, "s")
  const client = new LocalIpcClient(endpoint)
  const seen: unknown[] = []
  let capabilities: string[] = []
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  websocket.on("connection", socket => socket.on("message", bytes => {
    const frame = JSON.parse(String(bytes))
    seen.push(frame.request)
    const response = "RelayStatus" in frame.request
      ? { RelayStatus: { status: { daemon_id: "home-1", machine_id: "machine-1", capabilities } } }
      : { result: "sent", admitted: capabilities.includes("disposable_worker_control_v1") }
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response, error: null }))
  }))
  t.after(async () => {
    client.destroy()
    for (const socket of websocket.clients) socket.terminate()
    await new Promise<void>(resolve => websocket.close(() => resolve()))
    await new Promise<void>(resolve => server.close(() => resolve()))
    await rm(scratch, { recursive: true, force: true })
  })
  await new Promise<void>(resolve => server.listen(endpoint, resolve))
  const request = { ReleaseDisposableWorker: { homeKernelId: "home-1", allocationId: "allocation-1" } }
  await assert.rejects(client.send(request), /does not support/)
  assert.equal(seen.length, 1)
  capabilities = ["disposable_worker_control_v1"]
  assert.deepEqual(await client.send(request), { result: "sent", admitted: true })
  capabilities = []
  await assert.rejects(client.send(request), /does not support/)
  assert.deepEqual(seen, [{ RelayStatus: null }, { RelayStatus: null }, request, { RelayStatus: null }])
})
