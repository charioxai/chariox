import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptInputHistoryRefreshController,
} from "./prompt-input-history-refresh-controller.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((next) => {
    resolve = next
  })
  return { promise, resolve }
}

function createHarness() {
  const timers: FakeTimer[] = []
  const refreshes: string[] = []
  const errors: Array<{ error: unknown; sessionId: string }> = []
  const completions: Array<ReturnType<typeof deferred>> = []
  const controller = createPromptInputHistoryRefreshController<FakeTimer>({
    delayMs: 1500,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    refreshHistory(sessionId) {
      refreshes.push(sessionId)
      const completion = deferred()
      completions.push(completion)
      return completion.promise
    },
    onRefreshError(error, sessionId) {
      errors.push({ error, sessionId })
    },
  })

  return { completions, controller, errors, refreshes, timers }
}

test("prompt input history refresh controller coalesces in-flight refreshes", async () => {
  const { completions, controller, refreshes } = createHarness()

  const first = controller.refresh("s1")
  const second = controller.refresh("s2")

  assert.equal(first, second)
  assert.deepEqual(refreshes, ["s1"])

  completions[0]?.resolve()
  await first

  const third = controller.refresh("s2")
  assert.notEqual(third, first)
  assert.deepEqual(refreshes, ["s1", "s2"])
  completions[1]?.resolve()
  await third
})

test("prompt input history refresh controller schedules one delayed refresh", async () => {
  const { completions, controller, refreshes, timers } = createHarness()

  controller.schedule("s1")
  controller.schedule("s2")

  assert.equal(timers.length, 1)
  assert.equal(timers[0]?.delayMs, 1500)

  timers[0]?.callback()
  assert.deepEqual(refreshes, ["s1"])
  completions[0]?.resolve()
  await Promise.resolve()
})

test("prompt input history refresh controller reports scheduled refresh failures", async () => {
  const timers: FakeTimer[] = []
  const error = new Error("nope")
  const errors: Array<{ error: unknown; sessionId: string }> = []
  const controller = createPromptInputHistoryRefreshController<FakeTimer>({
    delayMs: 1500,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    async refreshHistory() {
      throw error
    },
    onRefreshError(errorValue, sessionId) {
      errors.push({ error: errorValue, sessionId })
    },
  })

  controller.schedule("s1")
  timers[0]?.callback()
  await new Promise<void>((resolve) => {
    setImmediate(resolve)
  })

  assert.deepEqual(errors, [{ error, sessionId: "s1" }])
})

test("prompt input history refresh controller clears a pending timer", () => {
  const { controller, timers } = createHarness()

  controller.schedule("s1")
  controller.clearTimer()

  assert.equal(timers[0]?.cleared, true)
})
