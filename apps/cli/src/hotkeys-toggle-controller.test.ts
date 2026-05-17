import assert from "node:assert/strict"
import test from "node:test"

import { createHotkeysToggleController } from "./hotkeys-toggle-controller.js"

test("hotkeys toggle controller ignores events already handled elsewhere", () => {
  const harness = createHarness()

  const handled = harness.controller.handle("keyboard", {
    name: "t",
    ctrl: true,
    defaultPrevented: true,
    preventDefault: () => {
      harness.prevented += 1
    },
  })

  assert.equal(handled, false)
  assert.equal(harness.prevented, 0)
  assert.equal(harness.toggled, 0)
  assert.deepEqual(harness.debugMessages, [])
  assert.deepEqual(harness.logs, [])
})

test("hotkeys toggle controller inspects but does not consume non-toggle modifier events", () => {
  const harness = createHarness()

  const handled = harness.controller.handle("stdin", {
    name: "x",
    ctrl: true,
  })

  assert.equal(handled, false)
  assert.equal(harness.toggled, 0)
  assert.equal(harness.logs.length, 1)
  assert.equal(harness.logs[0]?.message, "evaluated hotkeys toggle shortcut")
  assert.equal(harness.logs[0]?.fields.matched, false)
  assert.equal(harness.logs[0]?.fields.reason, "non_toggle_key")
})

test("hotkeys toggle controller toggles matched shortcuts and records focus state", () => {
  const harness = createHarness({
    savedFocus: { type: "textarea" },
    focus: { id: "prompt" },
  })

  const handled = harness.controller.handle("textarea", {
    name: "t",
    ctrl: true,
    preventDefault: () => {
      harness.prevented += 1
    },
    stopPropagation: () => {
      harness.stopped += 1
    },
  })

  assert.equal(handled, true)
  assert.equal(harness.hotkeysOpen, true)
  assert.equal(harness.toggled, 1)
  assert.equal(harness.prevented, 1)
  assert.equal(harness.stopped, 1)
  assert.deepEqual(harness.debugMessages, [
    "shortcut textarea matched reason=ctrl+t open=false key=t",
    "shortcut textarea finished open=true saved=textarea",
  ])
  assert.equal(harness.logs.length, 3)
  assert.equal(harness.logs[1]?.message, "toggling hotkeys via shortcut")
  assert.deepEqual(harness.logs[1]?.fields.current_focus, {
    described: { id: "prompt" },
  })
  assert.equal(harness.logs[2]?.fields.saved_focus, harness.savedFocus)
})

function createHarness(options: {
  savedFocus?: { type?: string | null } | null
  focus?: unknown
} = {}) {
  const harness = {
    hotkeysOpen: false,
    toggled: 0,
    prevented: 0,
    stopped: 0,
    debugMessages: [] as string[],
    logs: [] as Array<{ message: string; fields: Record<string, unknown> }>,
    savedFocus: options.savedFocus ?? null,
    focus: options.focus ?? null,
    controller: null as ReturnType<typeof createHotkeysToggleController> | null,
  }
  harness.controller = createHotkeysToggleController({
    hotkeysOpen: () => harness.hotkeysOpen,
    toggleHotkeys: () => {
      harness.toggled += 1
      harness.hotkeysOpen = !harness.hotkeysOpen
    },
    debugHotkey: (message) => {
      harness.debugMessages.push(message)
    },
    logDebug: (message, fields) => {
      harness.logs.push({ message, fields })
    },
    currentFocus: () => harness.focus,
    describeFocus: (focus) => ({ described: focus }),
    savedFocusDebug: () => harness.savedFocus,
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createHotkeysToggleController>
  }
}
