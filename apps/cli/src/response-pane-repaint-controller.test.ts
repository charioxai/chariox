import assert from "node:assert/strict"
import test from "node:test"

import { createResponsePaneRepaintController } from "./response-pane-repaint-controller.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
}

function createHarness() {
  const timers: FakeTimer[] = []
  let repaintCount = 0
  const controller = createResponsePaneRepaintController<FakeTimer>({
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs }
      timers.push(timer)
      return timer
    },
    repaint() {
      repaintCount += 1
    },
  })

  return { controller, repaintCount: () => repaintCount, timers }
}

test("response pane repaint controller repaints immediately and on the next tick", () => {
  const { controller, repaintCount, timers } = createHarness()

  controller.refreshFocus()

  assert.equal(repaintCount(), 1)
  assert.equal(timers[0]?.delayMs, 0)

  timers[0]?.callback()
  assert.equal(repaintCount(), 2)
})

test("response pane repaint controller ignores stale delayed focus refreshes", () => {
  const { controller, repaintCount, timers } = createHarness()

  controller.refreshFocus()
  controller.refreshFocus()

  assert.equal(repaintCount(), 2)

  timers[0]?.callback()
  assert.equal(repaintCount(), 2)

  timers[1]?.callback()
  assert.equal(repaintCount(), 3)
})
