import assert from "node:assert/strict"
import test from "node:test"

import { createTurnCompletionController } from "./turn-completion-controller.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function createHarness(options: { activeTurnWork?: boolean } = {}) {
  let now = 0
  let activeTurnWork = options.activeTurnWork ?? false
  let completions = 0
  const timers: FakeTimer[] = []
  const controller = createTurnCompletionController<FakeTimer>({
    now: () => now,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    hasActiveTurnWork: () => activeTurnWork,
    getDelayMs: (lastActivityAt) => Math.max(0, 300 - Math.max(0, now - lastActivityAt)),
    completeTurn: () => {
      completions += 1
    },
  })

  return {
    controller,
    timers,
    advanceTo(value: number) {
      now = value
    },
    completions: () => completions,
    setActiveTurnWork(value: boolean) {
      activeTurnWork = value
    },
  }
}

test("turn completion controller waits for a quiet window after activity", () => {
  const harness = createHarness()

  harness.controller.recordActivity()
  harness.controller.confirmAndSchedule()
  assert.equal(harness.timers[0]?.delayMs, 300)

  harness.advanceTo(100)
  harness.timers[0]?.callback()
  assert.equal(harness.completions(), 0)
  assert.equal(harness.timers[1]?.delayMs, 200)

  harness.advanceTo(300)
  harness.timers[1]?.callback()
  assert.equal(harness.completions(), 1)
  assert.equal(harness.controller.isConfirmed(), false)
})

test("turn completion controller defers scheduling while turn work is active", () => {
  const harness = createHarness({ activeTurnWork: true })

  harness.controller.confirmAndSchedule()
  assert.equal(harness.timers.length, 0)

  harness.setActiveTurnWork(false)
  harness.controller.maybeScheduleConfirmed()
  assert.equal(harness.timers[0]?.delayMs, 300)
})

test("turn completion controller cancels pending work while provider activity is live", () => {
  const harness = createHarness()

  harness.controller.confirmAndSchedule()
  harness.controller.handleProviderActivity(true)
  assert.equal(harness.timers[0]?.cleared, true)
  assert.equal(harness.controller.isConfirmed(), true)

  harness.controller.handleProviderActivity(false)
  assert.equal(harness.timers[1]?.delayMs, 300)
})

test("turn completion controller reset clears confirmation and pending timers", () => {
  const harness = createHarness()

  harness.controller.confirmAndSchedule()
  harness.controller.reset()
  harness.timers[0]?.callback()

  assert.equal(harness.timers[0]?.cleared, true)
  assert.equal(harness.completions(), 0)
  assert.equal(harness.controller.isConfirmed(), false)
})
