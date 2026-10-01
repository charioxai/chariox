import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { execFile } from "node:child_process"
import { currentOwnedPids, readOwnedProcessMetrics, sleep, stopProcessGroup } from "./room-coordinated-load-tui-process.mjs"
import { roomTuiPtyInvocation } from "./room-tui-pty.mjs"

const exec = promisify(execFile)
test("PTY descendant groups are measured and stopped with their owned wrapper", { skip: process.platform !== "linux" }, async () => {
  const invocation = roomTuiPtyInvocation([process.execPath, "-e", "setInterval(() => {}, 1000)"])
  const child = spawn(invocation.command, invocation.args, { detached: true, stdio: "ignore" })
  const tasks = [{ kind: "tui", processGroupId: child.pid }]
  let descendantTasks = []
  try {
    let pids = []
    for (let attempt = 0; attempt < 100; attempt++) {
      pids = await currentOwnedPids(tasks)
      if (pids.length >= 2) break
      await sleep(20)
    }
    assert.ok(pids.length >= 2, "script and its live Node child must both be attributed")
    const { stdout } = await exec("ps", ["-o", "pgid=", "-p", pids.join(",")])
    descendantTasks = [...new Set(stdout.trim().split(/\s+/).map(Number))]
      .map(processGroupId => ({ kind: "tui", processGroupId }))
    assert.ok(descendantTasks.length > 1, "the actual PTY creates a separate child group")
    const metrics = await readOwnedProcessMetrics(tasks)
    assert.ok(metrics.count >= 2)
    assert.ok(metrics.rssBytes > 10 * 1024 * 1024)
  } finally { await stopProcessGroup(child.pid, child) }
  assert.deepEqual(await currentOwnedPids(descendantTasks), [])
})
