import assert from "node:assert/strict"
import test from "node:test"

import { HOTKEY_TOGGLE_LABEL, isHotkeysToggleEvent, shouldCycleFocusOnTabEvent } from "./hotkeys.js"

test("isHotkeysToggleEvent matches Ctrl+T", () => {
  assert.equal(isHotkeysToggleEvent({ name: "t", ctrl: true }, "linux"), true)
})

test("isHotkeysToggleEvent matches kitty base code for uppercase names", () => {
  assert.equal(isHotkeysToggleEvent({ name: "T", ctrl: true, baseCode: 116 }, "linux"), true)
})

test("isHotkeysToggleEvent accepts macOS command shortcut", () => {
  assert.equal(isHotkeysToggleEvent({ name: "t", meta: true }, "darwin"), true)
  assert.equal(isHotkeysToggleEvent({ name: "t", meta: true }, "linux"), false)
})

test("isHotkeysToggleEvent ignores key releases", () => {
  assert.equal(isHotkeysToggleEvent({ name: "t", ctrl: true, eventType: "release" }, "linux"), false)
})

test("HOTKEY_TOGGLE_LABEL always shows Ctrl+T", () => {
  assert.equal(HOTKEY_TOGGLE_LABEL, "Ctrl+T")
})

test("shouldCycleFocusOnTabEvent suppresses Tab focus cycling while command center owns slash input", () => {
  assert.equal(shouldCycleFocusOnTabEvent({ name: "tab" }, {
    attached: true,
    hotkeysOpen: false,
    promptFocused: true,
    commandCenterOpen: true,
    commandCenterQuery: "/workflow node",
  }), false)

  assert.equal(shouldCycleFocusOnTabEvent({ name: "tab" }, {
    attached: true,
    hotkeysOpen: false,
    promptFocused: true,
    commandCenterOpen: false,
    commandCenterQuery: "/workflow node add ",
  }), false)

  assert.equal(shouldCycleFocusOnTabEvent({ name: "tab" }, {
    attached: true,
    hotkeysOpen: false,
    promptFocused: true,
    commandCenterOpen: false,
    commandCenterQuery: "regular prompt",
  }), true)
})
