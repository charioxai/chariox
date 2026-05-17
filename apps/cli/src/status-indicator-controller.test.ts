import assert from "node:assert/strict"
import test from "node:test"

import type { FocusedStatusBadge } from "./session-chrome-state.js"
import { createStatusIndicatorController } from "./status-indicator-controller.js"

test("status indicator controller logs and renders attached badges", () => {
  const badge = focusedBadge("WORKING")
  const logged: FocusedStatusBadge[] = []
  const rendered: Array<{
    box: string | undefined
    attached: boolean
    badge: FocusedStatusBadge | null
    animationFrame: number
  }> = []
  const controller = createStatusIndicatorController<string>({
    isAttached: () => true,
    getBadge: () => badge,
    getAnimationFrame: () => 3,
    resetFocusedBadgeChange: () => {
      throw new Error("should not reset while rendering a badge")
    },
    logFocusedBadgeChange: (value) => {
      logged.push(value)
    },
    renderIndicator: (options) => {
      rendered.push(options)
    },
  })

  controller.assignBox("status-box")
  controller.render()

  assert.deepEqual(logged, [badge])
  assert.deepEqual(rendered, [{ box: "status-box", attached: true, badge, animationFrame: 3 }])
})

test("status indicator controller clears badge state when detached or badge-less", () => {
  let attached = false
  let resetCount = 0
  const rendered: Array<{ attached: boolean; badge: FocusedStatusBadge | null }> = []
  const controller = createStatusIndicatorController({
    isAttached: () => attached,
    getBadge: () => focusedBadge("IGNORED"),
    getAnimationFrame: () => 0,
    resetFocusedBadgeChange: () => {
      resetCount += 1
    },
    logFocusedBadgeChange: () => {
      throw new Error("should not log when no badge is rendered")
    },
    renderIndicator: ({ attached: nextAttached, badge }) => {
      rendered.push({ attached: nextAttached, badge })
    },
  })

  controller.render()
  attached = true
  const badgeLessController = createStatusIndicatorController({
    isAttached: () => true,
    getBadge: () => null,
    getAnimationFrame: () => 0,
    resetFocusedBadgeChange: () => {
      resetCount += 1
    },
    logFocusedBadgeChange: () => {
      throw new Error("should not log without a badge")
    },
    renderIndicator: ({ attached: nextAttached, badge }) => {
      rendered.push({ attached: nextAttached, badge })
    },
  })
  badgeLessController.render()

  assert.equal(resetCount, 2)
  assert.deepEqual(rendered, [
    { attached: false, badge: null },
    { attached: true, badge: null },
  ])
})

function focusedBadge(label: string): FocusedStatusBadge {
  return {
    label,
    tone: "working",
    parts: [{ label, tone: "working" }],
  }
}
