import assert from "node:assert/strict"
import test from "node:test"

import { createWorkingAnimationController } from "./working-animation-controller.js"

test("working animation always advances its frame", () => {
  const harness = createHarness()

  harness.controller.tick()

  assert.deepEqual(harness.calls(), ["frame"])
})

test("working animation repaints session chrome while working", () => {
  const harness = createHarness({ sessionStatusMode: "working" })

  harness.controller.tick()

  assert.deepEqual(harness.calls(), ["frame", "chrome"])
})

test("working animation repaints split pane footers when split mode is enabled", () => {
  const harness = createHarness({ splitAgentResponseMode: true })

  harness.controller.tick()

  assert.deepEqual(harness.calls(), ["frame", "footers"])
})

test("working animation starts and stops one interval", () => {
  const harness = createHarness()

  harness.controller.start()
  harness.controller.start()
  harness.controller.stop()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), ["interval:120", "clear:timer-1"])
})

function createHarness(options: {
  sessionStatusMode?: string
  splitAgentResponseMode?: boolean
} = {}) {
  const calls: string[] = []
  const controller = createWorkingAnimationController<string>({
    intervalMs: 120,
    scheduleInterval: (_callback, intervalMs) => {
      calls.push(`interval:${intervalMs}`)
      return "timer-1"
    },
    clearInterval: (handle) => {
      calls.push(`clear:${handle}`)
    },
    incrementFrame: () => {
      calls.push("frame")
    },
    sessionStatusMode: () => options.sessionStatusMode ?? "idle",
    splitAgentResponseMode: () => options.splitAgentResponseMode ?? false,
    updateSessionChrome: () => {
      calls.push("chrome")
    },
    renderSplitPaneFooters: () => {
      calls.push("footers")
    },
  })

  return {
    controller,
    calls: () => calls,
  }
}
