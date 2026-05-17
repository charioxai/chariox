import assert from "node:assert/strict"
import test from "node:test"

import { createPromptFocusRetentionController } from "./prompt-focus-retention-controller.js"

test("prompt focus retention is idle while detached", () => {
  const harness = createHarness({ attached: false })

  harness.controller.retainFocus()

  assert.deepEqual(harness.calls(), [])
})

test("prompt focus retention schedules focus while attached", () => {
  const harness = createHarness({ attached: true })

  harness.controller.retainFocus()

  assert.deepEqual(harness.calls(), ["timer:0"])
  harness.fire()
  assert.deepEqual(harness.calls(), ["timer:0", "focus"])
})

function createHarness(options: { attached: boolean }) {
  const calls: string[] = []
  let callback: (() => void) | null = null
  const controller = createPromptFocusRetentionController<string>({
    delayMs: 0,
    scheduleTimer: (nextCallback, delayMs) => {
      callback = nextCallback
      calls.push(`timer:${delayMs}`)
      return "timer-1"
    },
    isAttached: () => options.attached,
    focusPromptInput: () => {
      calls.push("focus")
    },
  })

  return {
    controller,
    calls: () => calls,
    fire: () => {
      callback?.()
    },
  }
}
