import assert from "node:assert/strict"
import test from "node:test"

import { createTerminalOutputRecordQueue } from "./terminal-output-record-queue.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function createHarness() {
  const timers: FakeTimer[] = []
  const processed: number[][] = []
  const queue = createTerminalOutputRecordQueue<FakeTimer, number>({
    delayMs: 25,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    processRecords(records) {
      processed.push(records)
    },
  })

  return { processed, queue, timers }
}

function createBudgetedHarness(maxPendingRecords = Number.POSITIVE_INFINITY) {
  const timers: FakeTimer[] = []
  const processed: number[][] = []
  const queue = createTerminalOutputRecordQueue<FakeTimer, number>({
    delayMs: 25,
    maxRecordsPerFlush: 2,
    maxPendingRecords,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    processRecords(records) {
      processed.push(records)
    },
  })

  return { processed, queue, timers }
}

test("terminal output record queue batches records on one timer", () => {
  const { processed, queue, timers } = createHarness()

  queue.queue([1])
  queue.queue([2, 3])

  assert.equal(timers.length, 1)
  assert.equal(timers[0]?.delayMs, 25)
  assert.equal(queue.pendingCount(), 3)
  assert.equal(queue.hasPendingFlush(), true)

  timers[0]?.callback()

  assert.deepEqual(processed, [[1, 2, 3]])
  assert.equal(queue.pendingCount(), 0)
  assert.equal(queue.hasPendingFlush(), false)
})

test("terminal output record queue flushes pending records immediately", () => {
  const { processed, queue, timers } = createHarness()

  queue.queue([1, 2])
  queue.flush()

  assert.equal(timers[0]?.cleared, true)
  assert.deepEqual(processed, [[1, 2]])
  assert.equal(queue.pendingCount(), 0)
})

test("terminal output record queue ignores empty batches", () => {
  const { processed, queue, timers } = createHarness()

  queue.queue([])
  queue.flush()

  assert.equal(timers.length, 0)
  assert.deepEqual(processed, [])
})

test("terminal output record queue can clear the timer while retaining records", () => {
  const { processed, queue, timers } = createHarness()

  queue.queue([1])
  queue.clearTimer()

  assert.equal(timers[0]?.cleared, true)
  assert.equal(queue.hasPendingFlush(), false)
  assert.equal(queue.pendingCount(), 1)
  assert.deepEqual(processed, [])

  queue.flush()
  assert.deepEqual(processed, [[1]])
})

test("terminal output record queue limits each flush and reschedules overflow", () => {
  const { processed, queue, timers } = createBudgetedHarness()

  queue.queue([1, 2, 3, 4, 5])

  assert.equal(timers.length, 1)
  timers[0]?.callback()

  assert.deepEqual(processed, [[1, 2]])
  assert.equal(queue.pendingCount(), 3)
  assert.equal(queue.hasPendingFlush(), true)
  assert.equal(timers.length, 2)

  timers[1]?.callback()
  timers[2]?.callback()

  assert.deepEqual(processed, [[1, 2], [3, 4], [5]])
  assert.equal(queue.pendingCount(), 0)
  assert.equal(queue.hasPendingFlush(), false)
})

test("terminal output record queue drains every budgeted batch synchronously", () => {
  const { processed, queue, timers } = createBudgetedHarness()

  queue.queue([1, 2, 3, 4, 5])
  queue.drain()

  assert.equal(timers[0]?.cleared, true)
  assert.deepEqual(processed, [[1, 2], [3, 4], [5]])
  assert.equal(queue.pendingCount(), 0)
  assert.equal(queue.hasPendingFlush(), false)
})

test("terminal output record queue processes oversized batches without dropping or reordering records", () => {
  const { processed, queue, timers } = createBudgetedHarness(4)

  queue.queue([1, 2, 3, 4, 5, 6, 7])

  assert.equal(queue.pendingCount(), 3)
  assert.deepEqual(processed, [[1, 2], [3, 4]])
  assert.equal(timers.filter((timer) => !timer.cleared).length, 1)
  queue.drain()
  assert.deepEqual(processed.flat(), [1, 2, 3, 4, 5, 6, 7])
  assert.equal(queue.hasPendingFlush(), false)
})

test("terminal output record queue bounds retained records when producers outrun timers", () => {
  const { processed, queue, timers } = createBudgetedHarness(4)
  const expected = Array.from({ length: 1000 }, (_, index) => index)

  for (const record of expected) {
    queue.queue([record])
    assert.ok(queue.pendingCount() <= 4)
    assert.equal(timers.filter((timer) => !timer.cleared).length, 1)
  }
  queue.drain()
  assert.deepEqual(processed.flat(), expected)
  assert.equal(queue.pendingCount(), 0)
  assert.equal(queue.hasPendingFlush(), false)
})

// MP-08 / MP-10 (panes R8, PANE-11): the first record after a quiet window is
// processed at once; records inside the window still coalesce on one timer.
test("terminal output record queue processes the first record of a quiet stream immediately", () => {
  const timers: FakeTimer[] = []
  const processed: number[][] = []
  let now = 1_000
  const queue = createTerminalOutputRecordQueue<FakeTimer, number>({
    delayMs: 25,
    nowMs: () => now,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    processRecords(records) {
      processed.push(records)
    },
  })

  queue.queue([1])
  assert.deepEqual(processed, [[1]])
  assert.equal(timers.length, 0)

  now += 5
  queue.queue([2])
  queue.queue([3])
  assert.deepEqual(processed, [[1]])
  assert.equal(timers.length, 1)
  timers[0]?.callback()
  assert.deepEqual(processed, [[1], [2, 3]])

  now += 100
  queue.queue([4])
  assert.deepEqual(processed, [[1], [2, 3], [4]])
})

// MP-08 / MP-10: a status record and its first text share the leading budget.
test("leading status and text are delivered together without unbounded bursts", () => {
  let now = 0
  const processed: string[][] = []
  const timers: FakeTimer[] = []
  const queue = createTerminalOutputRecordQueue<FakeTimer, string>({
    delayMs: 48, nowMs: () => now, maxRecordsPerFlush: 2, maxPendingRecords: 4,
    scheduleTimer(callback, delayMs) { const timer = {callback, delayMs, cleared: false}; timers.push(timer); return timer },
    clearTimer(timer) { timer.cleared = true },
    processRecords(records) { processed.push(records) },
  })
  queue.queue(["status", "first text", "next text"])
  assert.deepEqual(processed, [["status", "first text"]])
  assert.equal(queue.pendingCount(), 1)
  queue.queue(["more text"])
  assert.equal(timers.filter(t => !t.cleared).length, 1)
  now = 48
  timers.at(-1)!.callback()
  assert.deepEqual(processed, [["status", "first text"], ["next text", "more text"]])
  queue.queue(["drained"])
  queue.drain()
  queue.queue(["same burst"])
  assert.equal(queue.pendingCount(), 1, "drain must update the quiet-window clock")
})
