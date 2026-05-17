import assert from "node:assert/strict"
import test from "node:test"

import { createPromptSurfaceMouseController } from "./prompt-surface-mouse-controller.js"

type TestMouseEvent = {
  button: "primary" | "secondary"
}

test("prompt surface mouse controller ignores non-primary buttons", () => {
  const harness = createHarness()

  harness.controller.handleMouseUp({ button: "secondary" })

  assert.deepEqual(harness.calls(), [])
})

test("prompt surface mouse controller schedules copy before focus retention", () => {
  const harness = createHarness()

  harness.controller.handleMouseUp({ button: "primary" })

  assert.deepEqual(harness.calls(), ["timer:0"])
  harness.fire()
  assert.deepEqual(harness.calls(), ["timer:0", "copy", "focus"])
})

function createHarness() {
  const calls: string[] = []
  let callback: (() => void) | null = null
  const controller = createPromptSurfaceMouseController<string, TestMouseEvent>({
    delayMs: 0,
    scheduleTimer: (nextCallback, delayMs) => {
      callback = nextCallback
      calls.push(`timer:${delayMs}`)
      return "timer-1"
    },
    isPrimaryButton: (event) => event.button === "primary",
    copySelection: () => {
      calls.push("copy")
    },
    retainPromptFocus: () => {
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
