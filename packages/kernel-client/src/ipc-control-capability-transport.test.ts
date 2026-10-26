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

// MP-08 / MP-10 / MP-11: verify the real shared transport refuses silent downgrade.
test("MP-08/MP-10/MP-11 exact cancellation admits main491 capability on the actual connected transport", async t => {
  const websocket = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => websocket.once("listening", resolve))
  const address = websocket.address()
  assert.ok(address && typeof address === "object")
  let capabilities: string[] = []
  const seen: unknown[] = []
  websocket.on("connection", socket => socket.on("message", bytes => {
    const frame = JSON.parse(String(bytes))
    seen.push(frame.request)
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, error: null,
      response: "RelayStatus" in frame.request
        ? { RelayStatus: { status: { daemon_id: "home-1", machine_id: "machine-1", capabilities } } }
        : { ok: true } }))
  }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}/kernel`)
  t.after(async () => {
    client.destroy()
    for (const socket of websocket.clients) socket.terminate()
    await new Promise<void>(resolve => websocket.close(() => resolve()))
  })
  const request = { CancelActivePrompt: { session_id: "s", attachment_id: "a", expected_prompt_id: "A" } }
  await assert.rejects(client.send(request), /does not support/)
  assert.deepEqual(seen, [{ RelayStatus: null }])
  capabilities = ["exact_prompt_cancellation_v499"]
  assert.deepEqual(await client.send(request), { ok: true })
  capabilities = []
  await assert.rejects(client.send(request), /does not support/)
  const legacy = { CancelActivePrompt: { session_id: "s", attachment_id: "a" } }
  assert.deepEqual(await client.send(legacy), { ok: true })
  assert.deepEqual(seen, [{ RelayStatus: null }, { RelayStatus: null }, request, { RelayStatus: null }, legacy])
})
