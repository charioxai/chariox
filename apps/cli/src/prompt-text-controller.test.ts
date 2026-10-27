import assert from "node:assert/strict"
import test from "node:test"

import { createPromptTextController } from "./prompt-text-controller.js"

type TestPromptInput = {
  plainText: string
  cursorOffset: number
  setText(value: string): void
  clear(): void
}

const createInput = (text = ""): TestPromptInput => ({
  plainText: text,
  cursorOffset: text.length,
  setText(value) {
    this.plainText = value
  },
  clear() {
    this.plainText = ""
  },
})

test("currentText and cursorOffset fall back to the snapshot without an input", () => {
  const controller = createPromptTextController({
    initialText: "draft",
    getPromptInput: () => null,
    refreshHighlights: () => {},
  })

  assert.equal(controller.currentText(), "draft")
  assert.equal(controller.cursorOffset(), "draft".length)

  controller.setText("next")

  assert.equal(controller.snapshot(), "next")
  assert.equal(controller.currentText(), "next")
  assert.equal(controller.cursorOffset(), "next".length)
})

test("setText updates prompt input, snapshot, cursor, and highlights while muted", () => {
  const input = createInput()
  const muteStates: boolean[] = []
  const controller = createPromptTextController({
    initialText: "",
    getPromptInput: () => input,
    refreshHighlights: () => {
      muteStates.push(controller.isProgrammaticMutation())
    },
  })

  controller.setText("hello")

  assert.equal(input.plainText, "hello")
  assert.equal(input.cursorOffset, 5)
  assert.equal(controller.snapshot(), "hello")
  assert.deepEqual(muteStates, [true])
  assert.equal(controller.isProgrammaticMutation(), false)
})

test("clear empties input and snapshot while muting content-change echoes", () => {
  const input = createInput("hello")
  const controller = createPromptTextController({
    initialText: "hello",
    getPromptInput: () => input,
    refreshHighlights: () => {},
  })

  controller.clear()

  assert.equal(input.plainText, "")
  assert.equal(input.cursorOffset, 0)
  assert.equal(controller.snapshot(), "")
  assert.equal(controller.isProgrammaticMutation(), false)
})

test("syncSnapshot mirrors current input text and setSnapshot updates only fallback state", () => {
  const input = createInput("input")
  const controller = createPromptTextController({
    initialText: "initial",
    getPromptInput: () => input,
    refreshHighlights: () => {},
  })

  assert.equal(controller.syncSnapshot(), "input")
  assert.equal(controller.snapshot(), "input")

  controller.setSnapshot("fallback")

  assert.equal(controller.snapshot(), "fallback")
  assert.equal(controller.currentText(), "input")
})
