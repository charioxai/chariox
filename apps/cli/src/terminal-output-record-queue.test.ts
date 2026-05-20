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

function createBudgetedHarness() {
  const timers: FakeTimer[] = []
  const processed: number[][] = []
  const queue = createTerminalOutputRecordQueue<FakeTimer, number>({
    delayMs: 25,
    maxRecordsPerFlush: 2,
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
