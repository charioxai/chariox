import assert from "node:assert/strict"
import test from "node:test"

import {
  createFooterFlashController,
  type FooterFlash,
} from "./footer-flash-controller.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function createHarness() {
  const timers: FakeTimer[] = []
  const flashes: Array<FooterFlash | null> = []
  let changeCount = 0
  const controller = createFooterFlashController<FakeTimer>({
    delayMs: 2500,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    setFooterFlash(flash) {
      flashes.push(flash)
    },
    onFooterFlashChange() {
      changeCount += 1
    },
  })

  return { changeCount: () => changeCount, controller, flashes, timers }
}

test("footer flash controller shows a flash and expires it", () => {
  const { changeCount, controller, flashes, timers } = createHarness()

  controller.flash("saved", "info")

  assert.deepEqual(flashes, [{ message: "saved", tone: "info" }])
  assert.equal(changeCount(), 1)
  assert.equal(timers[0]?.delayMs, 2500)

  timers[0]?.callback()

  assert.deepEqual(flashes, [{ message: "saved", tone: "info" }, null])
  assert.equal(changeCount(), 2)
})

test("footer flash controller replaces pending flashes", () => {
  const { controller, flashes, timers } = createHarness()

  controller.flash("first", "info")
  controller.flash("second", "error")

  assert.equal(timers[0]?.cleared, true)
  assert.deepEqual(flashes, [
    { message: "first", tone: "info" },
    { message: "second", tone: "error" },
  ])
})

test("footer flash controller can clear the pending timer", () => {
  const { controller, flashes, timers } = createHarness()

  controller.flash("saved", "info")
  controller.clearTimer()

  assert.equal(timers[0]?.cleared, true)
  assert.deepEqual(flashes, [{ message: "saved", tone: "info" }])
})
