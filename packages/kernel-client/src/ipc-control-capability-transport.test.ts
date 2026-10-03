import assert from "node:assert/strict"
import test from "node:test"
import net from "node:net"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { LocalIpcClient } from "./ipc.js"
import { guardedControlSessionRequest } from "./local-socket-session.js"

// WebSocket behavior is covered against real sockets in ipc-control-generation.test.ts.
  test("public Unix control requests require fresh capability on the admitted socket", async t => {
    const scratch = await mkdtemp(join(tmpdir(), "cx-control-"))
    const endpoint = join(scratch, "s")
    const client = new LocalIpcClient(endpoint)
    const seen: unknown[] = []
    let capabilities: string[] = []
    let connections = 0
    const sockets = new Set<net.Socket>()
    const server = net.createServer(socket => {
      connections++
      sockets.add(socket)
      socket.on("close", () => sockets.delete(socket))
      socket.on("error", () => {})
      let buffered = Buffer.alloc(0)
      let admitted = false
      socket.on("data", chunk => {
        buffered = Buffer.concat([buffered, chunk])
        while (buffered.length >= 4 && buffered.length >= buffered.readUInt32BE(0) + 4) {
          const size = buffered.readUInt32BE(0)
          const request = JSON.parse(buffered.subarray(4, size + 4).toString())
          buffered = buffered.subarray(size + 4)
          seen.push(request)
          const probe = JSON.stringify(request) === JSON.stringify(guardedControlSessionRequest)
          const response = probe
            ? { session: { version: 1 }, error: null, response: { RelayStatus: { status: {
              daemon_id: "home-1", machine_id: "machine-1", capabilities,
            } } } }
            : { error: null, response: { result: "sent", admitted } }
          if (probe) admitted = capabilities.includes("disposable_worker_control_v1")
          const body = Buffer.from(JSON.stringify(response))
          const frame = Buffer.alloc(body.length + 4)
          frame.writeUInt32BE(body.length)
          body.copy(frame, 4)
          socket.write(frame)
        }
      })
    })
    t.after(async () => {
      client.destroy()
      for (const socket of sockets) socket.destroy()
      await new Promise<void>(resolve => server.close(() => resolve()))
      await rm(scratch, { recursive: true, force: true })
    })
    await new Promise<void>(resolve => server.listen(endpoint, resolve))
    const request = { ReleaseDisposableWorker: { homeKernelId: "home-1", allocationId: "allocation-1" } }
    await assert.rejects(client.send(request), /does not support/)
    assert.deepEqual(seen, [guardedControlSessionRequest])
    capabilities = ["disposable_worker_control_v1"]
    assert.deepEqual(await client.send(request), { result: "sent", admitted: true })
    assert.deepEqual(seen, [guardedControlSessionRequest, guardedControlSessionRequest, request])
    capabilities = []
    await assert.rejects(client.send(request), /does not support/)
    assert.equal(seen.length, 4)
    assert.equal(connections, 3)
  })
