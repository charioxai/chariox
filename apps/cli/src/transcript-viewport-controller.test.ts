import assert from "node:assert/strict"
import test from "node:test"

import { createTranscriptViewportController } from "./transcript-viewport-controller.js"

test("transcript viewport controller scrolls to the bottom and records the final scroll top", () => {
  const harness = viewportHarness({ scrollHeight: 120, height: 30, scrollLeft: 4 })

  assert.equal(harness.controller.scrollToBottom(), true)

  assert.deepEqual(harness.scrolls, [{ x: 4, y: 90 }])
  assert.equal(harness.cancelCount, 1)
  assert.equal(harness.renderCount, 1)
  assert.equal(harness.lastScrollTop, 90)
})

test("transcript viewport controller clamps bottom scroll when content is shorter than the viewport", () => {
  const harness = viewportHarness({ scrollHeight: 20, height: 30 })

  assert.equal(harness.controller.scrollToBottom(), true)

  assert.deepEqual(harness.scrolls, [{ x: 0, y: 0 }])
  assert.equal(harness.lastScrollTop, 0)
})

test("transcript viewport controller is idle without a mounted scrollbox", () => {
  const harness = viewportHarness({ mounted: false })

  assert.equal(harness.controller.scrollToBottom(), false)

  assert.equal(harness.cancelCount, 0)
  assert.deepEqual(harness.scrolls, [])
  assert.equal(harness.renderCount, 0)
  assert.equal(harness.lastScrollTop, null)
})

function viewportHarness(options: {
  mounted?: boolean
  scrollHeight?: number
  height?: number
  scrollLeft?: number
} = {}) {
  const scrolls: Array<{ x: number; y: number }> = []
  let scrollTop = 0
  const harness = {
    scrolls,
    cancelCount: 0,
    renderCount: 0,
    lastScrollTop: null as number | null,
    controller: null as ReturnType<typeof createTranscriptViewportController> | null,
  }
  harness.controller = createTranscriptViewportController({
    getScrollbox: () => options.mounted === false
      ? null
      : {
          scrollHeight: options.scrollHeight ?? 100,
          height: options.height ?? 40,
          scrollLeft: options.scrollLeft ?? 0,
          get scrollTop() {
            return scrollTop
          },
          scrollTo(position) {
            scrolls.push(position)
            scrollTop = position.y
          },
          requestRender() {
            harness.renderCount += 1
          },
        },
    cancelHistoryScrollRestore: () => {
      harness.cancelCount += 1
    },
    setLastTranscriptScrollTop: (nextScrollTop) => {
      harness.lastScrollTop = nextScrollTop
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createTranscriptViewportController>
  }
}
