import assert from "node:assert/strict"
import test from "node:test"

import {
  captureCliDialogFocus,
  describeCliDialogFocusTarget,
  resolveCliDialogFocusTarget,
  restoreCliDialogFocus,
  type CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"

class FakeFocusTarget implements CliDialogFocusTarget {
  isDestroyed = false
  focused = false
  focusCount = 0
  blurCount = 0

  constructor(readonly id: string) {}

  focus(): void {
    this.focused = true
    this.focusCount += 1
  }

  blur(): void {
    this.focused = false
    this.blurCount += 1
  }
}

test("dialog focus target prefers current focus and falls back to prompt focus", () => {
  const current = new FakeFocusTarget("current")
  const prompt = new FakeFocusTarget("prompt")

  assert.equal(resolveCliDialogFocusTarget(current, prompt), current)
  current.isDestroyed = true
  assert.equal(resolveCliDialogFocusTarget(current, prompt), prompt)
  prompt.isDestroyed = true
  assert.equal(resolveCliDialogFocusTarget(current, prompt), null)
})

test("dialog focus capture blurs the selected target", () => {
  const current = new FakeFocusTarget("current")
  current.focused = true

  assert.equal(captureCliDialogFocus(current, null), current)
  assert.equal(current.focused, false)
  assert.equal(current.blurCount, 1)
})

test("dialog focus restore skips destroyed targets", () => {
  const target = new FakeFocusTarget("target")

  assert.equal(restoreCliDialogFocus(target), true)
  assert.equal(target.focusCount, 1)
  target.isDestroyed = true
  assert.equal(restoreCliDialogFocus(target), false)
  assert.equal(target.focusCount, 1)
})

test("dialog focus debug snapshot describes target state", () => {
  const target = new FakeFocusTarget("target")
  target.focused = true

  assert.deepEqual(describeCliDialogFocusTarget(target), {
    id: "target",
    type: "FakeFocusTarget",
    destroyed: false,
    focused: true,
  })
  assert.equal(describeCliDialogFocusTarget(null), null)
})
