import assert from "node:assert/strict"
import test, { mock } from "node:test"
import childProcess from "node:child_process"
import { watchCliLauncherParent } from "./cli-launcher-parent.js"

test("launcher parent watch exits after parent death without signalling another PID", async () => {
  let parent = 12345
  let exits = 0
  const stop = watchCliLauncherParent("12345", { exit: () => { exits += 1 }, inspect: () => parent === 12345, platform: "darwin" })
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
  const stop = watchCliLauncherParent("12345", { exit: () => { exits += 1 }, inspect: (pid) => {
    assert.equal(pid, 12345)
    return false
  }, platform: "darwin" })
  await new Promise((resolve) => setImmediate(resolve))
  stop()
  assert.equal(exits, 1)
})

test("launcher parent watch detects death before entry point starts", async () => {
  let exits = 0
  const stop = watchCliLauncherParent("12345", { exit: () => { exits += 1 }, inspect: () => false, platform: "darwin" })
  await new Promise((resolve) => setImmediate(resolve))
  stop()
  assert.equal(exits, 1)
})

test("direct Bun starts need no launcher and unsafe launcher PIDs are rejected", () => {
  watchCliLauncherParent(undefined, { exit: () => assert.fail("direct start exited") })()
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
    const stop = watchCliLauncherParent(String(process.ppid), { exit: () => { exits += 1 }, platform: "darwin" })
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
    stop = watchCliLauncherParent(String(process.ppid), { exit: () => assert.fail("healthy launcher exited"), platform: "darwin" })
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
  const stop = watchCliLauncherParent("12345", { exit: () => assert.fail("healthy Linux launcher exited"), inspect: () => { probes += 1; return true }, platform: "linux" })
  stop()
  assert.equal(probes, 0)
})

test("slow inspections never overlap and stopping ignores an in-flight result", async () => {
  let inspections = 0
  let finish!: (alive: boolean) => void
  let exits = 0
  const stop = watchCliLauncherParent("12345", { exit: () => { exits += 1 }, inspect: () => {
    inspections += 1
    return new Promise<boolean>((resolve) => { finish = resolve })
  }, platform: "darwin" })
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

// MP-08/MP-11: inspection failure cannot turn a live session into an orphan exit.
for (const code of ["ENOENT", "EAGAIN", "ENOMEM"]) {
  test(`inspection ${code} is inconclusive`, async () => {
    const error = Object.assign(new Error(code), { code, signal: null })
    const sync = mock.method(childProcess, "execFileSync", () => { throw error })
    const probe = mock.method(childProcess, "execFile", (...args: any[]) => {
      args.at(-1)(error, "")
      return {} as any
    })
    let exits = 0
    const stop = watchCliLauncherParent(String(process.ppid), { exit: () => { exits += 1 }, platform: "darwin" })
    try {
      await new Promise((resolve) => setImmediate(resolve))
      assert.equal(exits, 0)
    } finally {
      stop()
      sync.mock.restore()
      probe.mock.restore()
    }
  })
}

test("a spawning Bun wrapper is valid launcher ancestry", async () => {
  const rows = `${process.pid} 22222 S\n22222 12345 S\n12345 1 S\n`
  const sync = mock.method(childProcess, "execFileSync", () => rows)
  const probe = mock.method(childProcess, "execFile", (...args: any[]) => {
    args.at(-1)(null, rows)
    return {} as any
  })
  let exits = 0
  const stop = watchCliLauncherParent("12345", { exit: () => { exits += 1 }, platform: "darwin" })
  try {
    await new Promise((resolve) => setImmediate(resolve))
    assert.equal(exits, 0)
  } finally {
    stop()
    sync.mock.restore()
    probe.mock.restore()
  }
})

for (const [label, rows, error, expectedExits] of [
  ["missing launcher", `${process.pid} 1 S\n`, { code: 1, signal: null }, 1],
  ["zombie launcher", `${process.pid} 12345 S\n12345 1 Z\n`, null, 1],
  ["recycled PID outside ancestry", `${process.pid} 22222 S\n22222 1 S\n12345 1 S\n`, null, 1],
  ["nonzero ps with healthy snapshot", `${process.pid} 12345 S\n12345 1 S\n`, { code: 1, signal: null }, 0],
  ["empty snapshot", "", null, 0],
  ["malformed snapshot", "ps: unsupported option", { code: 1, signal: null }, 0],
  ["missing self row", "12345 1 S\n", null, 0],
  ["incomplete wrapper ancestry", `${process.pid} 22222 S\n12345 1 S\n`, null, 0],
  ["timed out snapshot", `${process.pid} 1 S\n`, { code: 1, killed: true, signal: "SIGTERM" }, 0],
] as const) {
  test(`process-table inspection: ${label}`, async () => {
    const probe = mock.method(childProcess, "execFile", (...args: any[]) => {
      assert.deepEqual(args[1], ["-axo", "pid=,ppid=,stat="])
      args.at(-1)(error, rows)
      return {} as any
    })
    let exits = 0
    const stop = watchCliLauncherParent("12345", { exit: () => { exits += 1 }, platform: "darwin" })
    try {
      await new Promise((resolve) => setImmediate(resolve))
      assert.equal(exits, expectedExits)
    } finally {
      stop()
      probe.mock.restore()
    }
  })
}

test("an inconclusive inspection retries and confirmed death exits only once", async () => {
  let probes = 0
  let exits = 0
  const stop = watchCliLauncherParent("12345", {
    platform: "darwin", exit: () => { exits += 1 },
    inspect: () => { if (++probes === 1) throw new Error("spawn EAGAIN"); return false },
  })
  try {
    await new Promise((resolve) => setTimeout(resolve, 2100))
    assert.equal(probes, 2)
    assert.equal(exits, 1)
  } finally { stop() }
})
