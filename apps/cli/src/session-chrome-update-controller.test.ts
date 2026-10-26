import assert from "node:assert/strict"
import test from "node:test"

import { createSessionChromeUpdateController } from "./session-chrome-update-controller.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function createHarness(options: { batched?: boolean } = {}) {
  let batched = options.batched ?? false
  let updates = 0
  let disposed = false
  const timers: FakeTimer[] = []
  const controller = createSessionChromeUpdateController<FakeTimer>({
    delayMs: 20,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    isBatched: () => batched,
    isDisposed: () => disposed,
    applyUpdate() {
      updates += 1
    },
  })

  return {
    controller,
    setDisposed(value: boolean) { disposed = value },
    setBatched(value: boolean) {
      batched = value
    },
    timers,
    updates: () => updates,
  }
}

test("session chrome update controller applies unthrottled requests immediately", () => {
  const { controller, timers, updates } = createHarness()

  controller.request(false)

  assert.equal(updates(), 1)
  assert.equal(timers.length, 0)
})

test("session chrome update controller coalesces throttled requests", () => {
  const { controller, timers, updates } = createHarness()

  controller.request(true)
  controller.request(true)

  assert.equal(timers.length, 1)
  assert.equal(timers[0]?.delayMs, 20)
  assert.equal(updates(), 0)

  timers[0]?.callback()
  assert.equal(updates(), 1)
})

test("session chrome update controller flushes a pending throttled update", () => {
  const { controller, timers, updates } = createHarness()

  controller.request(true)
  controller.flush()

  assert.equal(timers[0]?.cleared, true)
  assert.equal(updates(), 1)
})

test("session chrome update controller defers requests during UI batches", () => {
  const { controller, setBatched, timers, updates } = createHarness({ batched: true })

  controller.request(false)
  controller.request(true)

  assert.equal(timers.length, 0)
  assert.equal(updates(), 0)

  setBatched(false)
  controller.flushDeferred()
  assert.equal(updates(), 1)
})

test("session chrome update controller keeps flushes deferred during UI batches", () => {
  const { controller, setBatched, timers, updates } = createHarness()

  controller.request(true)
  setBatched(true)
  controller.flush()

  assert.equal(timers[0]?.cleared, true)
  assert.equal(updates(), 0)

  setBatched(false)
  controller.flushDeferred()
  assert.equal(updates(), 1)
})

// MP-08/MP-10/MP-11 A01: a pending room update cannot render after TUI teardown.
test("session chrome skips pending and new updates after renderer destruction", () => {
  const { controller, setDisposed, timers, updates } = createHarness()
  controller.request(true)
  setDisposed(true)
  timers[0]?.callback()
  controller.flushDeferred()
  controller.flush()
  controller.request(false)
  controller.request(true)
  assert.equal(updates(), 0)
  assert.equal(timers.length, 1)
})
