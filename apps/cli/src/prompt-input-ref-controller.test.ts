import assert from "node:assert/strict"
import test from "node:test"

import { createPromptInputRefController, type PromptInputRefRenderable } from "./prompt-input-ref-controller.js"

test("prompt input ref controller owns mounted prompt input helpers", () => {
  const input: PromptInputRefRenderable & { focusCount: number; blurCount: number; clearCount: number } = {
    height: 4,
    focused: true,
    plainText: "hello",
    syntaxStyle: null,
    focusCount: 0,
    blurCount: 0,
    clearCount: 0,
    focus() {
      this.focusCount += 1
    },
    blur() {
      this.blurCount += 1
    },
    clear() {
      this.clearCount += 1
    },
  }
  const controller = createPromptInputRefController<typeof input>()

  assert.equal(controller.current(), undefined)
  assert.equal(controller.currentOrNull(), null)
  assert.equal(controller.hasInput(), false)
  assert.equal(controller.height(1), 1)
  assert.equal(controller.isFocused(), false)
  assert.equal(controller.plainText(), undefined)

  controller.assignInput(input)
  controller.setSyntaxStyle("syntax")
  controller.focus()
  controller.blur()
  controller.clear()

  assert.equal(controller.current(), input)
  assert.equal(controller.currentOrNull(), input)
  assert.equal(controller.hasInput(), true)
  assert.equal(controller.height(1), 4)
  assert.equal(controller.isFocused(), true)
  assert.equal(controller.plainText(), "hello")
  assert.equal(input.syntaxStyle, "syntax")
  assert.equal(input.focusCount, 1)
  assert.equal(input.blurCount, 1)
  assert.equal(input.clearCount, 1)
})

test("prompt input ref controller drops a destroyed prompt input", () => {
  const input: PromptInputRefRenderable & { focusCount: number; blurCount: number; clearCount: number } = {
    height: 4,
    focused: true,
    plainText: "hello",
    syntaxStyle: "before",
    isDestroyed: false,
    focusCount: 0,
    blurCount: 0,
    clearCount: 0,
    focus() {
      this.focusCount += 1
    },
    blur() {
      this.blurCount += 1
    },
    clear() {
      this.clearCount += 1
    },
  }
  const controller = createPromptInputRefController<typeof input>()

  controller.assignInput(input)
  input.isDestroyed = true

  assert.equal(controller.current(), undefined)
  assert.equal(controller.currentOrNull(), null)
  assert.equal(controller.hasInput(), false)
  assert.equal(controller.height(1), 1)
  assert.equal(controller.isFocused(), false)
  assert.equal(controller.plainText(), undefined)

  controller.setSyntaxStyle("after")
  controller.focus()
  controller.blur()
  controller.clear()

  assert.equal(input.syntaxStyle, "before")
  assert.equal(input.focusCount, 0)
  assert.equal(input.blurCount, 0)
  assert.equal(input.clearCount, 0)
})
