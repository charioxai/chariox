import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcClient } from "./ipc.js"

// WebSocket behavior is covered against real sockets in ipc-control-generation.test.ts.
for (const endpoint of ["/tmp/unused-chariox-capability-test.sock"]) {
  test(`control requests require connected kernel capability on ${endpoint}`, async () => {
    const client = new LocalIpcClient(endpoint)
    const seen: unknown[] = []
    let capabilities: string[] = []
    const transport = async (request: unknown) => {
      seen.push(request)
      if (JSON.stringify(request) === JSON.stringify({ RelayStatus: null })) {
        return { RelayStatus: { status: { daemon_id: "home-1", machine_id: "machine-1", capabilities } } }
      }
      return { result: "sent" }
    }
    // Stub only transport I/O; exercise the real public send entry point.
    Object.assign(client, { sendLocalSocket: transport, sendWebSocket: transport })
    const request = { ReleaseDisposableWorker: { homeKernelId: "home-1", allocationId: "allocation-1" } }
    await assert.rejects(client.send(request), /does not support/)
    assert.deepEqual(seen, [{ RelayStatus: null }])
    capabilities = ["disposable_worker_control_v1"]
    assert.deepEqual(await client.send(request), { result: "sent" })
    assert.deepEqual(seen, [{ RelayStatus: null }, { RelayStatus: null }, request])
    capabilities = []
    await assert.rejects(client.send(request), /does not support/)
    assert.equal(seen.length, 4)
    client.destroy()
  })
}
