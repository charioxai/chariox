import assert from "node:assert/strict"
import test from "node:test"

import { createGlobalKeyboardShortcutController } from "./global-keyboard-shortcut-controller.js"

test("global keyboard shortcut controller delegates hotkey toggles first", () => {
  const harness = createHarness({ hotkeysHandled: true })
  const event = keyEvent("t")

  assert.equal(harness.controller.handleKey(event), true)

  assert.equal(event.prevented, false)
  assert.deepEqual(harness.calls(), ["hotkeys:t"])
})

test("global keyboard shortcut controller closes dialog overlays on escape", () => {
  const harness = createHarness({ dialogOpen: true })
  const event = keyEvent("escape")

  assert.equal(harness.controller.handleKey(event), true)

  assert.equal(event.prevented, true)
  assert.equal(event.stopped, true)
  assert.deepEqual(harness.calls(), ["hotkeys:escape", "close-dialog"])
})

test("global keyboard shortcut controller exits on ctrl-e", () => {
  const harness = createHarness()
  const event = keyEvent("e", { ctrl: true })

  assert.equal(harness.controller.handleKey(event), true)

  assert.equal(event.prevented, true)
  assert.deepEqual(harness.calls(), ["hotkeys:e", "exit"])
})

test("global keyboard shortcut controller maps ctrl-c to stop while turn work is active", () => {
  const stopHarness = createHarness({ activeTurnWork: true })
  assert.equal(stopHarness.controller.handleKey(keyEvent("c", { ctrl: true })), true)
  assert.deepEqual(stopHarness.calls(), ["hotkeys:c", "stop"])

  const exitHarness = createHarness({ activeTurnWork: false })
  assert.equal(exitHarness.controller.handleKey(keyEvent("c", { ctrl: true })), true)
  assert.deepEqual(exitHarness.calls(), ["hotkeys:c", "exit"])
})

test("global keyboard shortcut controller maps SIGINT to stop while turn work is active", () => {
  const stopHarness = createHarness({ activeTurnWork: true })
  stopHarness.controller.handleSigint()
  assert.deepEqual(stopHarness.calls(), ["stop"])

  const exitHarness = createHarness({ activeTurnWork: false })
  exitHarness.controller.handleSigint()
  assert.deepEqual(exitHarness.calls(), ["exit"])
})

test("global keyboard shortcut controller consumes keys while a dialog is open", () => {
  const harness = createHarness({ dialogOpen: true })
  const event = keyEvent("x")

  assert.equal(harness.controller.handleKey(event), true)

  assert.equal(event.prevented, true)
  assert.deepEqual(harness.calls(), ["hotkeys:x"])
})

test("global keyboard shortcut controller ignores unrelated keys", () => {
  const harness = createHarness()

  assert.equal(harness.controller.handleKey(keyEvent("x")), false)
  assert.deepEqual(harness.calls(), ["hotkeys:x"])
})

function createHarness(options: {
  hotkeysHandled?: boolean
  dialogOpen?: boolean
  activeTurnWork?: boolean
} = {}) {
  const calls: string[] = []
  const controller = createGlobalKeyboardShortcutController({
    handleHotkeysToggleShortcut: (_source, event) => {
      calls.push(`hotkeys:${event.name}`)
      return options.hotkeysHandled ?? false
    },
    dialogOverlayOpen: () => options.dialogOpen ?? false,
    closeActiveDialogOverlay: () => {
      calls.push("close-dialog")
    },
    requestExit: () => {
      calls.push("exit")
    },
    requestPromptStop: () => {
      calls.push("stop")
    },
    hasActiveTurnWork: () => options.activeTurnWork ?? false,
  })

  return {
    controller,
    calls: () => calls,
  }
}

function keyEvent(name: string, options: { ctrl?: boolean } = {}) {
  const event: {
    name: string
    ctrl?: boolean
    prevented: boolean
    stopped: boolean
    preventDefault: () => void
    stopPropagation: () => void
  } = {
    name,
    prevented: false,
    stopped: false,
    preventDefault: () => {
      event.prevented = true
    },
    stopPropagation: () => {
      event.stopped = true
    },
  }
  if (options.ctrl !== undefined) {
    event.ctrl = options.ctrl
  }
  return event
}
