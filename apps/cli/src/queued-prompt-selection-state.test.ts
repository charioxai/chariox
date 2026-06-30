import assert from "node:assert/strict"
import test from "node:test"

import {
  nextQueuedPromptSelectionId,
  selectedQueuedPromptId,
  selectedQueuedPromptIndex,
} from "./queued-prompt-selection-state.js"

test("queued prompt selection defaults to the first available prompt", () => {
  const items = prompts("prompt-1", "prompt-2")

  assert.equal(selectedQueuedPromptIndex(items, null), 0)
  assert.equal(selectedQueuedPromptId(items, null), "prompt-1")
})

test("queued prompt selection preserves a still-visible prompt", () => {
  const items = prompts("prompt-1", "prompt-2")

  assert.equal(selectedQueuedPromptIndex(items, "prompt-2"), 1)
  assert.equal(selectedQueuedPromptId(items, "prompt-2"), "prompt-2")
})

test("queued prompt selection falls back when the selected prompt leaves the queue", () => {
  const items = prompts("prompt-2")

  assert.equal(selectedQueuedPromptIndex(items, "prompt-1"), 0)
  assert.equal(selectedQueuedPromptId(items, "prompt-1"), "prompt-2")
})

test("queued prompt selection cycles through available prompts", () => {
  const items = prompts("prompt-1", "prompt-2", "prompt-3")

  assert.equal(nextQueuedPromptSelectionId(items, "prompt-1", 1), "prompt-2")
  assert.equal(nextQueuedPromptSelectionId(items, "prompt-1", -1), "prompt-3")
  assert.equal(nextQueuedPromptSelectionId(items, "missing", 1), "prompt-2")
})

test("queued prompt selection handles an empty queue", () => {
  assert.equal(selectedQueuedPromptIndex([], "prompt-1"), -1)
  assert.equal(selectedQueuedPromptId([], "prompt-1"), null)
  assert.equal(nextQueuedPromptSelectionId([], "prompt-1", 1), null)
})

function prompts(...ids: string[]): Array<{ promptId: string }> {
  return ids.map((promptId) => ({ promptId }))
}
