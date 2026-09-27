import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { mkdtemp, rm } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import test from "node:test"
import { captureRoomKernelDiagnostics } from "./room-kernel-diagnostics.mjs"

test("captures an actual kernel connection failure before disposable state is removed", {
  skip: !process.env.CHARIOX_DIAGNOSTIC_KERNEL_BINARY,
  timeout: 15000,
}, async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-kernel-evidence-live-"))
  const reservation = net.createServer()
  const sockets = new Set()
  // Reserve an unreachable WebSocket service without racing another listener.
  reservation.on("connection", (socket) => { sockets.add(socket); socket.destroy() })
  reservation.listen(0, "127.0.0.1")
  await once(reservation, "listening")
  const relayUrl = `ws://127.0.0.1:${reservation.address().port}`
  const logs = path.join(root, "logs")
  let kernel
  try {
    const ports = []
    for (let i = 0; i < 4; i++) {
      const server = net.createServer()
      server.listen(0, "127.0.0.1")
      await once(server, "listening")
      ports.push(server.address().port)
      await new Promise((resolve) => server.close(resolve))
    }
    kernel = spawn(process.env.CHARIOX_DIAGNOSTIC_KERNEL_BINARY, [], {
      cwd: root,
      env: {
        ...process.env,
        CHARIOX_HOME: path.join(root, "home"),
        CHARIOX_LOG_DIR: logs,
        CHARIOX_LOG_LEVEL: "info",
        TOKIO_WORKER_THREADS: "1",
        CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]),
        CHARIOX_CODEX_PORT: String(ports[2]), CHARIOX_OPENCODE_PORT: String(ports[3]),
        CHARIOX_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        CHARIOX_DAEMON_ID: "kernel-evidence-test",
        CHARIOX_MACHINE_ID: "kernel-evidence-machine",
        CHARIOX_RELAY_URL: relayUrl,
        CHARIOX_RELAY_TOKEN: "LOCAL-TEST-TOKEN-MUST-NOT-LEAK",
        CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
        XDG_CONFIG_HOME: path.join(root, "config"),
        XDG_STATE_HOME: path.join(root, "state"),
        XDG_CACHE_HOME: path.join(root, "cache"),
      },
      stdio: "ignore",
    })
    await once(kernel, "spawn")
    const deadline = Date.now() + 8000
    let result
    do {
      result = await captureRoomKernelDiagnostics(logs, { primary: relayUrl })
      if (result.events.some((event) => event.event === "relay socket connect failed")) break
      await sleep(100)
    } while (Date.now() < deadline && kernel.exitCode === null)
    assert.ok(result.events.some((event) => event.event === "relay socket connect failed" && event.relay === "primary"),
      `missing actual connection failure: ${JSON.stringify(result)}`)
    assert.doesNotMatch(JSON.stringify(result), /LOCAL-TEST|127\.0\.0\.1|kernel-evidence-machine/)
  } finally {
    if (kernel?.pid && kernel.exitCode === null && kernel.signalCode === null) {
      const exited = once(kernel, "exit")
      kernel.kill("SIGTERM")
      const force = setTimeout(() => kernel.kill("SIGKILL"), 2000)
      await exited.finally(() => clearTimeout(force))
    }
    for (const socket of sockets) socket.destroy()
    await new Promise((resolve) => reservation.close(resolve))
    await rm(root, { recursive: true, force: true })
  }
})
