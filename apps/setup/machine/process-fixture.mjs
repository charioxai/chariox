// Drill helpers for tests that start their own local processes.
import { createServer } from "node:net"
import { once } from "node:events"
export async function availablePort() {
  const s = createServer(); await new Promise(r => s.listen(0, "127.0.0.1", r))
  const p = s.address().port; await new Promise(r => s.close(r)); return p
}
export async function stopOwned(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing unsafe drill PID")
  const stopped = once(child, "close")
  child.kill("SIGTERM")
  await stopped
}
