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
    assert.equal(controller.handleRendererKey({ name: "f7" }), true)
    assert.equal(renderer.useMouse, false)
    assert.equal(controller.isActive(), true)
    assert.equal(clears, 1)
    assert.deepEqual(hints, [NATIVE_SELECTION_HINT])
    assert.equal(controller.handleRendererKey({ name: exit, eventType: "release" }), true)
    assert.equal(controller.isActive(), true)
    assert.equal(controller.handleRendererKey({ name: exit }), true)
    assert.equal(renderer.useMouse, initial)
    assert.equal(controller.isActive(), false)
    assert.deepEqual(hints, [NATIVE_SELECTION_HINT, null])
  }
})

test("MP-08 / MP-10 native selection consumes app keys; disposal restores mouse configuration", () => {
  const renderer = { useMouse: true, clearSelection: () => {} }
  const controller = createNativeSelectionController({ renderer, setHint: () => {} })
  controller.handleRendererKey({ name: "f7" })
  for (const key of [{ name: "enter" }, { name: "c", ctrl: true }, { name: "y" }]) {
    assert.equal(controller.handleRendererKey(key), true)
  }
  assert.equal(controller.handleRendererKey({ name: "e", ctrl: true }), false)
  controller.dispose()
  assert.equal(renderer.useMouse, true)
  assert.equal(controller.handleRendererKey({ name: "enter" }), false)
})

test("MP-08 / MP-10 raw replay never toggles twice and handoff discards skipped decisions", () => {
  const renderer = { useMouse: true, clearSelection: () => {} }
  const controller = createNativeSelectionController({ renderer, setHint: () => {} })
  const toggle = { name: "f7", sequence: "\x1b[18~" }
  assert.equal(controller.handleRendererKey(toggle), true)
  assert.equal(controller.handleKey(toggle), true)
  assert.equal(controller.isActive(), true)
  assert.equal(controller.handleRendererKey(toggle), true)
  assert.equal(controller.handleKey(toggle), true)
  assert.equal(controller.isActive(), false)
  controller.handleRendererKey({ name: "return", sequence: "\r" })
  controller.discardInput()
  controller.handleRendererKey(toggle)
  assert.equal(controller.handleKey(toggle), true)
  assert.equal(controller.isActive(), true)
  // Raw terminal responses that the renderer consumes do not steal ownership.
  assert.equal(controller.handleKey({ name: "", sequence: "\x1b[?997;1n" }), true)
  controller.handleRendererKey(toggle)
  assert.equal(controller.handleKey(toggle), true)
  assert.equal(controller.isActive(), false)
  controller.dispose()
})
