import assert from "node:assert/strict"
import test from "node:test"

import {
  createTranscriptHistoryAutoloadController,
  type TranscriptHistoryAutoloadScrollbox,
} from "./transcript-history-autoload-controller.js"

test("monitorScroll loads older history when the user scrolls to the top", () => {
  let loads = 0
  let lastScrollTop = 12
  const scrollbox: TranscriptHistoryAutoloadScrollbox = {
    height: 80,
    scrollHeight: 240,
    scrollTop: 0,
  }
  const controller = createTranscriptHistoryAutoloadController({
    scheduleTimer: () => {},
    getScrollbox: () => scrollbox,
    isScrollRestoring: () => false,
    isAttached: () => true,
    isLoadingHistory: () => false,
    hasMoreHistory: () => true,
    getLastScrollTop: () => lastScrollTop,
    setLastScrollTop: (scrollTop) => {
      lastScrollTop = scrollTop
    },
    loadOlderHistory: () => {
      loads += 1
    },
  })

  controller.monitorScroll()

  assert.equal(loads, 1)
  assert.equal(lastScrollTop, 0)
})

test("monitorScroll stays idle while scroll restoration is active", () => {
  let loads = 0
  let lastScrollTop = 12
  const controller = createTranscriptHistoryAutoloadController({
    scheduleTimer: () => {},
    getScrollbox: () => ({ height: 80, scrollHeight: 240, scrollTop: 0 }),
    isScrollRestoring: () => true,
    isAttached: () => true,
    isLoadingHistory: () => false,
    hasMoreHistory: () => true,
    getLastScrollTop: () => lastScrollTop,
    setLastScrollTop: (scrollTop) => {
      lastScrollTop = scrollTop
    },
    loadOlderHistory: () => {
      loads += 1
    },
  })

  controller.monitorScroll()

  assert.equal(loads, 0)
  assert.equal(lastScrollTop, 12)
})

test("scheduleShortViewportCheck loads when the attached viewport is not filled", () => {
  const timers: Array<() => void> = []
  let loads = 0
  const controller = createTranscriptHistoryAutoloadController({
    scheduleTimer: (callback) => {
      timers.push(callback)
    },
    getScrollbox: () => ({ height: 80, scrollHeight: 40, scrollTop: 0 }),
    isScrollRestoring: () => false,
    isAttached: () => true,
    isLoadingHistory: () => false,
    hasMoreHistory: () => true,
    getLastScrollTop: () => 0,
    setLastScrollTop: () => {},
    loadOlderHistory: () => {
      loads += 1
    },
  })

  controller.scheduleShortViewportCheck()
  assert.equal(loads, 0)

  timers.shift()?.()

  assert.equal(loads, 1)
})

test("scheduleShortViewportCheck rechecks short viewports after loading a page", async () => {
  const timers: Array<() => void> = []
  let loads = 0
  const controller = createTranscriptHistoryAutoloadController({
    scheduleTimer: (callback) => {
      timers.push(callback)
    },
    getScrollbox: () => ({ height: 80, scrollHeight: 40, scrollTop: 0 }),
    isScrollRestoring: () => false,
    isAttached: () => true,
    isLoadingHistory: () => false,
    hasMoreHistory: () => true,
    getLastScrollTop: () => 0,
    setLastScrollTop: () => {},
    loadOlderHistory: () => {
      loads += 1
      return loads === 1
    },
  })

  controller.scheduleShortViewportCheck()
  timers.shift()?.()
  await Promise.resolve()

  assert.equal(loads, 1)
  assert.equal(timers.length, 1)

  timers.shift()?.()
  await Promise.resolve()

  assert.equal(loads, 2)
  assert.equal(timers.length, 0)
})

test("maybeLoadForShortViewport stays idle when detached or already loading", () => {
  let loads = 0
  const controller = createTranscriptHistoryAutoloadController({
    scheduleTimer: () => {},
    getScrollbox: () => ({ height: 80, scrollHeight: 40, scrollTop: 0 }),
    isScrollRestoring: () => false,
    isAttached: () => false,
    isLoadingHistory: () => true,
    hasMoreHistory: () => true,
    getLastScrollTop: () => 0,
    setLastScrollTop: () => {},
    loadOlderHistory: () => {
      loads += 1
    },
  })

  controller.maybeLoadForShortViewport()

  assert.equal(loads, 0)
})
