import assert from "node:assert/strict"
import { mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
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

test("CLI transport gates fresh WebSocket and Unix advertisements and preserves current, stale and other endpoints", async context => {
  const root = mkdtempSync(join(tmpdir(), "chariox-feature-minimum-"))
  const previous = process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
  process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = root
  context.after(() => {
    if (previous === undefined) delete process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
    else process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR = previous
    rmSync(root, { recursive: true, force: true })
  })
  const requests: unknown[] = []
  context.mock.method(KernelClient.prototype, "send", async (request: unknown) => {
    requests.push(request)
    if (request && typeof request === "object" && "RelayStatus" in request) return { RelayStatus: { status: { daemon_id: "k" } } }
    return { AppHostActionAccepted: { operation_id: "o", action: { kind: "clipboard_write", text: "copy" } } }
  })
  const publish = (version: number, heartbeat = Date.now()) => writeFileSync(join(root, "kernel.json"), JSON.stringify({
    schema_version: 1, kernel_id: "k", machine_id: "m", host: "127.0.0.1", port: 43121,
    heartbeat_at_ms: heartbeat, local_daemon_protocol_version: version,
  }))
  const client = new LocalIpcClient("ws://127.0.0.1:43121/kernel")
  context.after(() => client.close())
  const request = { AcceptAppHostAction: { session_id: "s", operation_id: "o" } }
  publish(388)
  await assert.rejects(client.send(request), /kernel too old: App host actions needs protocol ≥409; this kernel is 388; update the kernel/)
  assert.deepEqual(requests, [])
  publish(409)
  await client.send(request)
  assert.deepEqual(requests, [request])
  publish(388, Date.now() - 60000)
  await client.send(request)
  publish(388)
  const other = new LocalIpcClient("ws://127.0.0.1:43122/kernel")
  context.after(() => other.close())
  await other.send(request)
  assert.equal(requests.length, 3)
  const unix = new LocalIpcClient(join(root, "kernel.sock"))
  context.after(() => unix.close())
  requests.length = 0
  await assert.rejects(unix.send(request), /App host actions needs protocol ≥409; this kernel is 388/)
  assert.deepEqual(requests, [{ RelayStatus: null }])
  publish(410)
  await unix.send(request)
  assert.deepEqual(requests, [{ RelayStatus: null }, { RelayStatus: null }, request])
})
