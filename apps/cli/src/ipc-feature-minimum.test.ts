import assert from "node:assert/strict"
import { mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:http"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient as KernelClient } from "@chariox/kernel-client/ipc"
import { LocalIpcClient } from "./ipc.js"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"
import { readRoomViewerAvailability } from "./room-viewer-availability.js"

test("Room viewer propagates an advertised-version refusal without parsing its wording", async () => {
  await assert.rejects(readRoomViewerAvailability({ send: async request => {
    requireKernelFeatureProtocol(request, 281)
    assert.fail("a version refusal must stop before the next request")
  } }, "s", "/room view", false), {
    message: "kernel too old: Room slice binding needs protocol ≥282; this kernel is 281; update the kernel",
  })
})

test("CLI transport gates fresh WebSocket advertisements and preserves current, stale and other endpoints", async context => {
  const root = mkdtempSync(join(tmpdir(), "chariox-feature-minimum-"))
  const previous = process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
  process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = root
  context.after(() => {
    if (previous === undefined) delete process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
    else process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = previous
    rmSync(root, { recursive: true, force: true })
  })
  let now = Date.now()
  context.mock.method(Date, "now", () => now)
  context.mock.method(KernelClient.prototype, "onKernelEvent", () => assert.fail("protocol caching must not add event consumers"))
  const clients: TestClient[] = []
  class TestClient extends LocalIpcClient {
    constructor(...args: ConstructorParameters<typeof KernelClient>) { super(...args); clients.push(this) }
    disconnect() { this.onControlConnectionChanged() }
  }
  const disconnect = () => clients.forEach(client => client.disconnect())
  const requests: unknown[] = []
  context.mock.method(KernelClient.prototype, "send", async (request: unknown) => {
    requests.push(request)
    if (request && typeof request === "object" && "RelayStatus" in request) assert.fail("protocol checks must not probe global state")
    return { AppHostActionAccepted: { operation_id: "o", action: { kind: "clipboard_write", text: "copy" } } }
  })
  const publish = (version: number, heartbeat = Date.now()) => writeFileSync(join(root, "kernel.json"), JSON.stringify({
    schema_version: 1, kernel_id: "k", machine_id: "m", host: "127.0.0.1", port: 43121,
    heartbeat_at_ms: heartbeat, local_daemon_protocol_version: version,
  }))
  const client = new TestClient("ws://127.0.0.1:43121/kernel")
  context.after(() => client.close())
  const request = { AcceptAppHostAction: { session_id: "s", operation_id: "o" } }
  publish(388)
  await assert.rejects(client.send(request), /kernel too old: App host actions needs protocol ≥409; this kernel is 388; update the kernel/)
  assert.deepEqual(requests, [])
  publish(409)
  await assert.rejects(client.send(request), /this kernel is 388/)
  now += 30001
  publish(409)
  await client.send(request)
  assert.deepEqual(requests, [request])
  disconnect()
  publish(388, Date.now() - 60000)
  await client.send(request)
  publish(388)
  const other = new TestClient("ws://127.0.0.1:43122/kernel")
  context.after(() => other.close())
  await other.send(request)
  assert.equal(requests.length, 3)
})


test("a control socket closing invalidates cached advertisements without event consumers", async context => {
  const root = mkdtempSync(join(tmpdir(), "chariox-feature-connection-"))
  const previous = process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
  process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = root
  context.after(() => {
    if (previous === undefined) delete process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
    else process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = previous
    rmSync(root, { recursive: true, force: true })
  })
  context.mock.method(KernelClient.prototype, "onKernelEvent", () => assert.fail("protocol caching must not add event consumers"))
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  server.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data)) as { request_id: string }
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
  }))
  let changes = 0
  let closed: () => void = () => {}
  class ConnectionClient extends LocalIpcClient {
    protected override onControlConnectionChanged(): void {
      super.onControlConnectionChanged()
      changes++
      if (changes === 2) closed()
    }
  }
  const client = new ConnectionClient(`ws://127.0.0.1:${address.port}/kernel`)
  context.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  const publish = (version: number) => writeFileSync(join(root, "kernel.json"), JSON.stringify({
    schema_version: 1, kernel_id: "k", machine_id: "m", host: "127.0.0.1", port: address.port,
    heartbeat_at_ms: Date.now(), local_daemon_protocol_version: version,
  }))
  const request = { AcceptAppHostAction: { session_id: "s", operation_id: "o" } }
  publish(410)
  await client.send(request)
  await client.send(request)
  publish(388)
  await client.send(request) // The advertisement is still cached on this connection.
  const disconnected = new Promise<void>(resolve => { closed = resolve })
  for (const socket of server.clients) socket.terminate()
  await disconnected
  await assert.rejects(client.send(request), /this kernel is 388/)
  assert.equal(changes, 2)
})


test("session-scoped Unix requests never probe global RelayStatus for a protocol advertisement", async context => {
  const root = mkdtempSync(join(tmpdir(), "chariox-feature-unix-"))
  const previous = process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
  process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = root
  writeFileSync(join(root, "kernel.json"), JSON.stringify({ schema_version: 1, kernel_id: "k",
    machine_id: "m", host: "127.0.0.1", port: 43121, heartbeat_at_ms: Date.now(), local_daemon_protocol_version: 1 }))
  const server = createServer()
  const sockets = new WebSocketServer({ server })
  const requests: unknown[] = []
  sockets.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data)) as { request_id: string; request: Record<string, unknown> }
    requests.push(frame.request)
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
      response: "RelayStatus" in frame.request ? null : { ok: true },
      error: "RelayStatus" in frame.request ? "Unix peer has no live authority for this request" : null }))
  }))
  const path = join(root, "kernel.sock")
  await new Promise<void>(resolve => server.listen(path, resolve))
  const client = new LocalIpcClient(path)
  context.after(async () => {
    client.destroy()
    for (const socket of sockets.clients) socket.terminate()
    await new Promise<void>(resolve => sockets.close(() => resolve()))
    await new Promise<void>(resolve => server.close(() => resolve()))
    if (previous === undefined) delete process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
    else process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = previous
    rmSync(root, { recursive: true, force: true })
  })
  const request = { GetRoomEnvironmentState: { session_id: "s" } }
  assert.deepEqual(await client.send(request), { ok: true })
  assert.deepEqual(requests, [request])
})
