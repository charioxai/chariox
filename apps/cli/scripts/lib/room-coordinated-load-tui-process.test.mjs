import assert from "node:assert/strict"
import { getEventListeners } from "node:events"
import { spawn } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { execFile } from "node:child_process"
import { performance } from "node:perf_hooks"
import { currentOwnedPids, readOwnedProcessMetrics, sleep, stopProcessGroup } from "./room-coordinated-load-tui-process.mjs"
import { roomTuiPtyInvocation } from "./room-tui-pty.mjs"

const exec = promisify(execFile)
test("coordinated load sleep preserves its minimum delay after an early timer wake", async (t) => {
  const originalSetTimeout = globalThis.setTimeout
  let timerCalls = 0
  let elapsed = 0
  const delays = []
  t.mock.method(performance, "now", () => elapsed)
  t.mock.method(globalThis, "setTimeout", (callback, delay, ...args) => {
    timerCalls++
    delays.push(delay)
    elapsed += timerCalls === 1 ? 0 : delay
    return originalSetTimeout(callback, 0, ...args)
  })
  const before = performance.now()
  await sleep(20)
  assert.ok(performance.now() - before >= 20, "a premature timer must not shorten the requested stall")
  assert.ok(timerCalls >= 2, "sleep must re-arm the remaining deadline after an early wake")
  assert.deepEqual(delays, [20, 20])
})

test("coordinated load sleep remains abortable after an early timer wake", async (t) => {
  const controller = new AbortController()
  const originalSetTimeout = globalThis.setTimeout
  let timerCalls = 0
  t.mock.method(performance, "now", () => 0)
  t.mock.method(globalThis, "setTimeout", (callback, delay, ...args) => {
    timerCalls++
    if (timerCalls === 2) queueMicrotask(() => controller.abort())
    return originalSetTimeout(callback, timerCalls === 1 ? 0 : delay, ...args)
  })
  await assert.rejects(sleep(100, controller.signal), /coordinated load interrupted/)
  assert.equal(timerCalls, 2)
  assert.equal(getEventListeners(controller.signal, "abort").length, 0)
})

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
