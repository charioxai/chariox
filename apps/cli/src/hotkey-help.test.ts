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

 test("waiting-room command shortcut is shown only with the App prototype flag", () => {
   const previous=process.env.CHARIOX_USER_APP_VIEWS_PROTOTYPE
   try {
     process.env.CHARIOX_USER_APP_VIEWS_PROTOTYPE="0"
     assert.ok(!buildHotkeySections(false).flatMap(s=>s.items).some(i=>i.keys==="/"))
     process.env.CHARIOX_USER_APP_VIEWS_PROTOTYPE="1"
     assert.ok(buildHotkeySections(false).flatMap(s=>s.items).some(i=>i.keys==="/"))
   } finally {
     if(previous===undefined)delete process.env.CHARIOX_USER_APP_VIEWS_PROTOTYPE
     else process.env.CHARIOX_USER_APP_VIEWS_PROTOTYPE=previous
   }
 })
