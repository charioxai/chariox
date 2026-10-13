import assert from "node:assert/strict"
import test, { mock } from "node:test"
import childProcess from "node:child_process"
import { watchCliLauncherParent } from "./cli-launcher-parent.js"

test("launcher parent watch exits after parent death without signalling another PID", async () => {
  let parent = 12345
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => parent, () => { exits += 1 }, () => true, "darwin")
  try {
    assert.equal(exits, 0)
    parent = 1
    await new Promise((resolve) => setTimeout(resolve, 1150))
    assert.equal(exits, 1)
  } finally {
    stop()
  }
})

test("launcher parent watch detects death when the runtime caches its parent PID", async () => {
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => 12345, () => { exits += 1 }, (pid) => {
    assert.equal(pid, 12345)
    return false
  }, "darwin")
  await new Promise((resolve) => setImmediate(resolve))
  stop()
  assert.equal(exits, 1)
})

test("launcher parent watch detects death before entry point starts", () => {
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => 1, () => { exits += 1 }, undefined, "darwin")
  stop()
  assert.equal(exits, 1)
})

test("direct Bun starts need no launcher and unsafe launcher PIDs are rejected", () => {
  watchCliLauncherParent(undefined, () => 1, () => assert.fail("direct start exited"))()
  for (const pid of ["0", "1", "-1", "NaN", "undefined", "2.5"]) {
    assert.throws(() => watchCliLauncherParent(pid), /invalid CLI launcher parent PID/)
  }
})

// MP-08/MP-11: a foreground-group signal may interrupt the inspection child.
test("an interrupted process inspection is retried instead of exiting the TUI", async () => {
  const probe = mock.method(childProcess, "execFile", (...args: any[]) => {
    args.at(-1)(Object.assign(new Error("inspection interrupted"), { signal: "SIGINT" }), "")
    return {} as any
  })
  let exits = 0
  try {
    const stop = watchCliLauncherParent(String(process.ppid), () => process.ppid, () => { exits += 1 }, undefined, "darwin")
    await new Promise((resolve) => setImmediate(resolve))
    stop()
    assert.equal(exits, 0)
  } finally {
    probe.mock.restore()
  }
})

// MP-08/MP-11: slow macOS process inspection must not stall rendering.
test("process inspection leaves render timers responsive", async () => {
  const rows = `${process.pid} ${process.ppid} S\n${process.ppid} 1 S\n`
  const sync = mock.method(childProcess, "execFileSync", () => {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 80)
    return rows
  })
  const asyncProbe = mock.method(childProcess, "execFile", (...args: any[]) => {
    setTimeout(() => args.at(-1)(null, rows), 80)
    return { unref() {} } as any
  })
  let stop = () => {}
  try {
    const started = performance.now()
    const frame = new Promise<number>((resolve) => setTimeout(() => resolve(performance.now() - started), 10))
    // The optional platform selects the macOS fallback on a Linux test host.
    stop = watchCliLauncherParent(String(process.ppid), () => process.ppid, () => assert.fail("healthy launcher exited"), undefined, "darwin")
    assert.ok(await frame < 50, "parent inspection blocked a render timer")
    assert.equal(sync.mock.callCount(), 0)
  } finally {
    stop()
    sync.mock.restore()
    asyncProbe.mock.restore()
  }
})

test("Linux relies on launcher PR_SET_PDEATHSIG without process inspection", () => {
  let probes = 0
  const stop = watchCliLauncherParent("12345", () => 12345, () => assert.fail("healthy Linux launcher exited"), () => { probes += 1; return true }, "linux")
  stop()
  assert.equal(probes, 0)
})

test("slow inspections never overlap and stopping ignores an in-flight result", async () => {
  let inspections = 0
  let finish!: (alive: boolean) => void
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => 12345, () => { exits += 1 }, () => {
    inspections += 1
    return new Promise<boolean>((resolve) => { finish = resolve })
  }, "darwin")
  try {
    await new Promise((resolve) => setTimeout(resolve, 1100))
    assert.equal(inspections, 1)
    stop()
    finish(false)
    await new Promise((resolve) => setImmediate(resolve))
    assert.equal(exits, 0)
  } finally {
    stop()
  }
})
