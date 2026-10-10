import assert from "node:assert/strict"
import test from "node:test"

import { createPromptSurfaceMouseController } from "./prompt-surface-mouse-controller.js"

type TestMouseEvent = {
  button: "primary" | "secondary"
  isDragging?: boolean
}

test("prompt surface mouse controller ignores non-primary buttons", () => {
  const harness = createHarness()

  harness.controller.handleMouseUp({ button: "secondary" })

  assert.deepEqual(harness.calls(), [])
})

test("MP-08 / MP-10 click release returns typing focus without copying an old selection", () => {
  // Focus never clears the renderer selection, so the highlight survives while
  // the next keystroke reaches the prompt instead of the transcript scrollbox.
  const harness = createHarness()
  harness.controller.handleMouseUp({ button: "primary" })
  harness.controller.handleSelection("old selection")
  assert.deepEqual(harness.calls(), ["focus"])
})

test("MP-08 / MP-10 completed selection defers only its released text", () => {
  const harness = createHarness()
  harness.controller.handleMouseUp({ button: "primary", isDragging: true })
  harness.controller.handleSelection("released text")
  assert.deepEqual(harness.calls(), ["focus", "timer:0"])
  harness.fire()
  assert.deepEqual(harness.calls(), ["focus", "timer:0", "copy:released text"])
  harness.controller.handleSelection("unrelated selection")
  assert.deepEqual(harness.calls(), ["focus", "timer:0", "copy:released text"])
})

test("MP-08 / MP-10 empty completed selection does not schedule a copy", () => {
  const harness = createHarness()
  harness.controller.handleMouseUp({ button: "primary", isDragging: true })
  harness.controller.handleSelection("")
  harness.controller.handleSelection("unrelated selection")
  assert.deepEqual(harness.calls(), ["focus"])
})

test("MP-08 / MP-10 a selection outside the prompt surface does not auto-copy", () => {
  const harness = createHarness()
  harness.controller.handleSelection("other surface")
  assert.deepEqual(harness.calls(), [])
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
    copyText: (text) => {
      calls.push(`copy:${text}`)
      return true
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
