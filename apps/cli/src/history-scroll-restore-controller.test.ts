import assert from "node:assert/strict"
import test from "node:test"

import {
  createHistoryScrollRestoreController,
  type HistoryScrollRestoreScrollbox,
} from "./history-scroll-restore-controller.js"

function createScrollbox(options: {
  scrollHeight: number
  scrollTop: number
  height: number
}): HistoryScrollRestoreScrollbox & { renderRequests: number } {
  return {
    height: options.height,
    scrollHeight: options.scrollHeight,
    scrollLeft: 0,
    scrollTop: options.scrollTop,
    renderRequests: 0,
    scrollTo(position) {
      this.scrollTop = position.y
    },
    requestRender() {
      this.renderRequests += 1
    },
  }
}

test("restorePrependedHistory preserves viewport after older transcript entries are prepended", async () => {
  const timers: Array<() => void> = []
  const scrollbox = createScrollbox({ scrollHeight: 220, scrollTop: 0, height: 80 })
  let lastScrollTop = 0
  const controller = createHistoryScrollRestoreController({
    scheduleTimer: (callback) => {
      timers.push(callback)
    },
    getScrollbox: () => scrollbox,
    setLastScrollTop: (scrollTop) => {
      lastScrollTop = scrollTop
    },
  })

  const restored = controller.restorePrependedHistory({
    scrollbox,
    previousScrollTop: 0,
    previousScrollHeight: 120,
    previousViewportHeight: 80,
  })

  assert.equal(controller.isRestoring(), true)
  assert.equal(timers.length, 1)

  timers.shift()?.()
  assert.equal(scrollbox.scrollTop, 100)
  assert.equal(controller.isRestoring(), true)

  timers.shift()?.()
  await restored

  assert.equal(scrollbox.scrollTop, 100)
  assert.equal(lastScrollTop, 100)
  assert.equal(controller.isRestoring(), false)
  assert.equal(scrollbox.renderRequests >= 3, true)
})

test("cancel stops a pending restore without applying stale scroll position", async () => {
  const timers: Array<() => void> = []
  const scrollbox = createScrollbox({ scrollHeight: 220, scrollTop: 0, height: 80 })
  let lastScrollTop = 0
  const controller = createHistoryScrollRestoreController({
    scheduleTimer: (callback) => {
      timers.push(callback)
    },
    getScrollbox: () => scrollbox,
    setLastScrollTop: (scrollTop) => {
      lastScrollTop = scrollTop
    },
  })

  const restored = controller.restorePrependedHistory({
    scrollbox,
    previousScrollTop: 0,
    previousScrollHeight: 120,
    previousViewportHeight: 80,
  })
  controller.cancel()

  timers.shift()?.()
  await restored

  assert.equal(scrollbox.scrollTop, 0)
  assert.equal(lastScrollTop, 0)
  assert.equal(controller.isRestoring(), false)
})
