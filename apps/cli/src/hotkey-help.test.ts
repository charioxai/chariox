import test from "node:test"
import assert from "node:assert/strict"

import { buildHotkeySections } from "./hotkey-help.js"
import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"

test("buildHotkeySections switches attached and waiting-room help", () => {
  assert.deepEqual(buildHotkeySections(true).map((section) => section.title), ["Global", "Session"])
  assert.deepEqual(buildHotkeySections(false).map((section) => section.title), ["Global", "Waiting room"])
})

test("buildHotkeySections keeps the global toggle label from hotkey matching", () => {
  assert.equal(buildHotkeySections(true)[0]?.items[0]?.keys, HOTKEY_TOGGLE_LABEL)
})
