import assert from "node:assert/strict"
import test from "node:test"
import { createKernelClock, kernelTimeFromHealth, approvalCountdown } from "./kernel-clock.js"

test("MP-08/MP-11 countdown follows fresh kernel time and monotonic elapsed, never the local wall clock", async () => {
  let monotonic = 10
  const clock = createKernelClock(async () => {monotonic = 90; return 1_000_000}, () => monotonic)
  await clock.synchronize()
  assert.equal(clock.now(), 1_000_040)
  const wallNow = Date.now
  try {
    Date.now = () => 9_000_000_000_000
    monotonic += 2_000
    assert.equal(clock.now(), 1_002_040)
  } finally {Date.now = wallNow}
  assert.equal(approvalCountdown(192_000), "3:12 left")
  clock.invalidate()
  assert.equal(clock.now(), null)
  assert.equal(kernelTimeFromHealth({DaemonHealth: {projection: {metadata: {generated_at_ms: 42}}}}), 42)
  assert.throws(() => kernelTimeFromHealth({}), /unavailable/)
})

test("MP-08/MP-11 an old connection cannot restore its clock after invalidation", async () => {
  let finish!: (value: number) => void
  const clock = createKernelClock(() => new Promise(resolve => {finish = resolve}))
  const old = clock.synchronize()
  clock.invalidate()
  finish(50_000)
  await old
  assert.equal(clock.now(), null)
})

// MP-08 / MP-11: a delayed health sample cannot undo an authoritative feed sample.
test("MP-08/MP-11 projected kernel time supersedes an in-flight health clock read", async () => {
  let finish!: (value: number) => void
  let mono = 100
  const clock = createKernelClock(() => new Promise(resolve => {finish = resolve}), () => mono)
  const read = clock.synchronize()
  clock.observe!(4_000)
  mono += 20
  finish(1_000)
  await read
  assert.equal(clock.now(), 4_020)
})
