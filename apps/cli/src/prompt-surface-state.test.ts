import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptPlaceholderSyncController,
} from "./prompt-surface-state.js"

test("prompt placeholder sync controller updates the mounted input only", () => {
  const input = { placeholder: "old" }
  const controller = createPromptPlaceholderSyncController({
    getPromptInput: () => input,
    getPlaceholder: () => "new",
  })

  controller.sync()

  assert.equal(input.placeholder, "new")

  createPromptPlaceholderSyncController({
    getPromptInput: () => null,
    getPlaceholder: () => "ignored",
  }).sync()
})
