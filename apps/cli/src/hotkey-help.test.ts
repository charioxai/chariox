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

test("session help lists approval opening keys and slash command", () => {
  const items = buildHotkeySections(true).flatMap(section => section.items)
  assert.ok(items.some(item => item.keys.includes("Ctrl+G") && item.keys.includes("F8") && item.keys.includes("/approvals")))
})

test("MP-08 / MP-10 copy help offers a legacy-terminal key distinct from Ctrl+C", () => {
  for (const attached of [true, false]) {
    const items = buildHotkeySections(attached).flatMap(section => section.items)
    assert.ok(items.some(item => item.keys.includes("F6") && item.description.includes("Copy")))
    assert.ok(!items.some(item => item.keys.includes("Ctrl+Shift+C")))
  }
})
