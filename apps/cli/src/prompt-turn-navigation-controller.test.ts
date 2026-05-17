import assert from "node:assert/strict"
import test from "node:test"

import { createPromptTurnNavigationController } from "./prompt-turn-navigation-controller.js"

test("prompt turn navigation ignores non-shift arrow keys and non-empty prompts", () => {
  const harness = createHarness()

  assert.equal(harness.controller.handleKey({ name: "up", eventType: "press" }), false)
  assert.equal(createHarness({ promptText: "draft" }).controller.handleKey({ name: "up", eventType: "press", shift: true }), false)
  assert.deepEqual(harness.scrolls(), [])
})

test("prompt turn navigation scrolls to the previous prompt anchor", () => {
  const harness = createHarness({ scrollTop: 30, promptOffsets: [0, 20, 50] })

  assert.equal(harness.controller.handleKey({ name: "up", eventType: "press", shift: true }), true)

  assert.deepEqual(harness.scrolls(), [{ x: 4, y: 0 }])
  assert.equal(harness.renderCount(), 1)
  assert.equal(harness.lastScrollTop(), 0)
})

test("prompt turn navigation scrolls to the next prompt anchor", () => {
  const harness = createHarness({ scrollTop: 30, promptOffsets: [0, 20, 50] })

  assert.equal(harness.controller.handleKey({ name: "down", eventType: "press", shift: true }), true)

  assert.deepEqual(harness.scrolls(), [{ x: 4, y: 50 }])
  assert.equal(harness.lastScrollTop(), 50)
})

test("prompt turn navigation consumes valid shortcuts even without a mounted scrollbox", () => {
  const harness = createHarness({ mounted: false })

  assert.equal(harness.controller.handleKey({ name: "down", eventType: "press", shift: true }), true)
  assert.deepEqual(harness.scrolls(), [])
})

function createHarness(options: {
  attached?: boolean
  promptText?: string
  promptOffsets?: number[]
  scrollTop?: number
  scrollLeft?: number
  mounted?: boolean
} = {}) {
  const scrolls: Array<{ x: number; y: number }> = []
  let renderCount = 0
  let lastScrollTop: number | null = null
  let currentScrollTop = options.scrollTop ?? 0
  const controller = createPromptTurnNavigationController({
    isAttached: () => options.attached ?? true,
    getPromptText: () => options.promptText ?? "",
    getPromptOffsets: () => options.promptOffsets ?? [0, 40],
    getScrollState: () => options.mounted === false
      ? null
      : { left: options.scrollLeft ?? 4, top: currentScrollTop },
    scrollTo: (position) => {
      scrolls.push(position)
      currentScrollTop = position.y
    },
    requestRender: () => {
      renderCount += 1
    },
    setLastTranscriptScrollTop: (scrollTop) => {
      lastScrollTop = scrollTop
    },
  })

  return {
    controller,
    scrolls: () => scrolls,
    renderCount: () => renderCount,
    lastScrollTop: () => lastScrollTop,
  }
}
