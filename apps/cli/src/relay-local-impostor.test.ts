import assert from "node:assert/strict"
import { once } from "node:events"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"
import { WebSocketServer } from "ws"

import { bootstrapCliRuntime } from "./cli-runtime-bootstrap.js"
import { LocalIpcClient } from "./ipc.js"
import { withWaitingRoomWorkspaceClient } from "./waiting-room-kernel-client.js"

// MP-08 / MP-11 F1: a fresh discovery record and a matching public ID do
// not authenticate whoever has taken over the kernel's loopback port.
for (const entry of ["relay launch", "waiting-room switch"] as const) {
  test(`MP-11 F1 ${entry} never probes a replacement TCP listener`, async (t) => {
    const root = mkdtempSync(path.join(tmpdir(), "chariox-mp11-impostor-"))
    const originalHome = process.env.CHARIOX_HOME
    process.env.CHARIOX_HOME = root
    const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
    await once(server, "listening")
    let connections = 0
    server.on("connection", (socket) => {
      connections++
      socket.on("message", (data) => {
        const frame = JSON.parse(data.toString())
        socket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
          response: { RelayStatus: { status: { daemon_id: "home-1" } } } }))
      })
    })
    t.after(async () => {
      for (const socket of server.clients) socket.terminate()
      await new Promise<void>((resolve) => server.close(() => resolve()))
      if (originalHome === undefined) delete process.env.CHARIOX_HOME
      else process.env.CHARIOX_HOME = originalHome
      rmSync(root, { recursive: true, force: true })
    })
    const address = server.address()
    assert.ok(address && typeof address !== "string")
    const registry = path.join(root, "kernels", "active")
    mkdirSync(registry, { recursive: true })
    writeFileSync(path.join(registry, "home-1.json"), JSON.stringify({
      schema_version: 1, kernel_id: "home-1", machine_id: "machine-1",
      host: "127.0.0.1", port: address.port, heartbeat_at_ms: Date.now(),
    }))
    if (entry === "relay launch") {
      const result = await bootstrapCliRuntime({
        argv: ["--detached", "--relay-url", "ws://127.0.0.1:1", "--relay-token", "fixture",
          "--target-daemon-id", "home-1"], cwd: root,
      })
      assert.equal(result.kind, "ready")
      if (result.kind !== "ready") throw new Error("detached bootstrap was not ready")
      t.after(() => result.bootstrap.client.close())
      assert.equal(connections, 0, "an unauthenticated listener received a local probe")
      assert.equal(result.kernelEndpoint, "ws://127.0.0.1:1")
    } else {
      const control = new LocalIpcClient("/unused.sock")
      const refused = new Error("authenticated control refused target")
      let resolved = false
      let read = false
      control.send = async () => { resolved = true; throw refused }
      t.after(() => control.close())
      await assert.rejects(withWaitingRoomWorkspaceClient(control, {
        kernelRef: "home-1", machineRef: "machine-1", clientId: "cli-1", isActive: () => true,
      }, async () => { read = true }), (error) => error === refused)
      assert.equal(connections, 0, "waiting room probed an unauthenticated local listener")
      assert.equal(resolved, true)
      assert.equal(read, false)
    }
  })
}
