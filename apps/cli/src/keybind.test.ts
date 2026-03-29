import assert from "node:assert/strict"
import test from "node:test"

import { keybindFromEvent, matchKeybind, parseKeybinds } from "./keybind.js"

test("keybindFromEvent normalizes kitty base code names", () => {
  assert.deepEqual(keybindFromEvent({ name: "T", ctrl: true, baseCode: 116 }), {
    name: "t",
    ctrl: true,
    meta: false,
    shift: false,
    super: false,
    leader: false,
  })
})

test("matchKeybind matches parsed ctrl+t bindings", () => {
  const binding = parseKeybinds("ctrl+t")[0]
  assert.equal(matchKeybind(binding, keybindFromEvent({ name: "t", ctrl: true })), true)
  assert.equal(matchKeybind(binding, keybindFromEvent({ name: "t", meta: true })), false)
})
