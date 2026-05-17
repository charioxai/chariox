import assert from "node:assert/strict"
import test from "node:test"

import {
  createConnectionHealthWatchdogController,
} from "./connection-health-watchdog-controller.js"

type FakeInterval = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function createHarness(options: { attached?: boolean; working?: boolean; closing?: boolean } = {}) {
  let now = 0
  let attached = options.attached ?? true
  let working = options.working ?? true
  let closing = options.closing ?? false
  const intervals: FakeInterval[] = []
  const recoveries: Array<{ nextConsecutiveSilentPolls: number; timeSinceLastActivityMs: number }> = []
  const controller = createConnectionHealthWatchdogController<FakeInterval>({
    now: () => now,
    intervalMs: 250,
    silenceWindowMs: 2000,
    silentThreshold: 3,
    scheduleInterval(callback, delayMs) {
      const interval = { callback, delayMs, cleared: false }
      intervals.push(interval)
      return interval
    },
    clearInterval(interval) {
      interval.cleared = true
    },
    isClosing: () => closing,
    isAttached: () => attached,
    isWorking: () => working,
    onRecover(decision) {
      recoveries.push({
        nextConsecutiveSilentPolls: decision.nextConsecutiveSilentPolls,
        timeSinceLastActivityMs: decision.timeSinceLastActivityMs,
      })
    },
  })

  return {
    controller,
    intervals,
    recoveries,
    setAttached(value: boolean) {
      attached = value
    },
    setClosing(value: boolean) {
      closing = value
    },
    setNow(value: number) {
      now = value
    },
    setWorking(value: boolean) {
      working = value
    },
  }
}

test("connection health watchdog starts only one interval", () => {
  const { controller, intervals } = createHarness()

  controller.start()
  controller.start()

  assert.equal(intervals.length, 1)
  assert.equal(intervals[0]?.delayMs, 250)
})

test("connection health watchdog stops when closing", () => {
  const { controller, intervals, setClosing } = createHarness()

  controller.start()
  setClosing(true)
  intervals[0]?.callback()

  assert.equal(intervals[0]?.cleared, true)
})

test("connection health watchdog recovers after repeated silent checks", () => {
  const { controller, recoveries, setNow } = createHarness()

  setNow(2501)
  controller.check()
  controller.check()
  controller.check()

  assert.deepEqual(recoveries, [{
    nextConsecutiveSilentPolls: 3,
    timeSinceLastActivityMs: 2501,
  }])
})

test("connection health watchdog activity resets silent checks", () => {
  const { controller, recoveries, setNow } = createHarness()

  setNow(2501)
  controller.check()
  controller.check()
  controller.recordActivity()
  setNow(2600)
  controller.check()

  assert.deepEqual(recoveries, [])
})

test("connection health watchdog ignores detached or idle states", () => {
  const { controller, recoveries, setAttached, setNow, setWorking } = createHarness()

  setNow(2501)
  setAttached(false)
  controller.check()
  setAttached(true)
  setWorking(false)
  controller.check()

  assert.deepEqual(recoveries, [])
})
