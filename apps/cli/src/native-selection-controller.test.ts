import assert from "node:assert/strict"
import test from "node:test"
import { createNativeSelectionController } from "./native-selection-controller.js"
import { NATIVE_SELECTION_HINT } from "./clipboard.js"

test("MP-08 / MP-10 F7 disables real mouse ownership, Escape/F7 restores the prior mode", () => {
  for (const initial of [true, false]) for (const exit of ["escape", "f7"]) {
    const hints: Array<string | null> = []
    let clears = 0
    const renderer = { useMouse: initial, clearSelection: () => { clears++ } }
    const controller = createNativeSelectionController({ renderer, setHint: hint => hints.push(hint) })
    assert.equal(controller.ownsRendererKey({ name: "f7" }), true)
    assert.equal(controller.handleKey({ name: "f7" }), true)
    assert.equal(renderer.useMouse, false)
    assert.equal(controller.isActive(), true)
    assert.equal(clears, 1)
    assert.deepEqual(hints, [NATIVE_SELECTION_HINT])
    assert.equal(controller.ownsRendererKey({ name: "escape" }), true)
    assert.equal(controller.handleKey({ name: exit, eventType: "release" }), true)
    assert.equal(controller.isActive(), true)
    assert.equal(controller.handleKey({ name: exit }), true)
    assert.equal(renderer.useMouse, initial)
    assert.equal(controller.isActive(), false)
    assert.deepEqual(hints, [NATIVE_SELECTION_HINT, null])
    assert.equal(controller.ownsRendererKey({ name: "escape" }), false)
  }
})

test("MP-08 / MP-10 native selection consumes app keys; disposal restores mouse configuration", () => {
  const renderer = { useMouse: true, clearSelection: () => {} }
  const controller = createNativeSelectionController({ renderer, setHint: () => {} })
  controller.handleKey({ name: "f7" })
  for (const key of [{ name: "enter" }, { name: "c", ctrl: true }, { name: "y" }]) {
    assert.equal(controller.ownsRendererKey(key), true)
    assert.equal(controller.handleKey(key), true)
  }
  assert.equal(controller.handleKey({ name: "e", ctrl: true }), false)
  controller.dispose()
  assert.equal(renderer.useMouse, true)
  assert.equal(controller.handleKey({ name: "enter" }), false)
})
