import assert from "node:assert/strict"
import test from "node:test"

import {
  createCliRendererFocusController,
} from "./cli-renderer-focus-controller.js"
import type {
  CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"

test("renderer focus controller reads the current focused renderable", () => {
  const first = focusTarget("first")
  const second = focusTarget("second")
  const renderer = {
    currentFocusedRenderable: first,
  }
  const controller = createCliRendererFocusController(renderer)

  assert.equal(controller.current(), first)

  renderer.currentFocusedRenderable = second

  assert.equal(controller.current(), second)
})

test("renderer focus controller describes focus targets for debug output", () => {
  const controller = createCliRendererFocusController({})
  const target = focusTarget("prompt")

  assert.deepEqual(controller.describe(target), {
    id: "prompt",
    type: "Object",
    destroyed: false,
    focused: true,
  })
  assert.equal(controller.describe(null), null)
})

function focusTarget(id: string): CliDialogFocusTarget {
  return {
    id,
    isDestroyed: false,
    focused: true,
    focus() {},
    blur() {},
  }
}
