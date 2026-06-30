import assert from "node:assert/strict"
import test from "node:test"

import {
  queuedPromptActionLabel,
  queuedPromptMetaLabel,
  queuedPromptTitleLabel,
} from "./queued-prompt-strip-labels.js"

test("queued prompt strip uses compact focused action labels", () => {
  assert.equal(queuedPromptTitleLabel(2, true), "QUEUE • 2 prompts • S steer • C cancel")
  assert.equal(queuedPromptActionLabel("steer", true), "S")
  assert.equal(queuedPromptActionLabel("cancel", true), "C")
})

test("queued prompt strip keeps unfocused mouse labels descriptive", () => {
  assert.equal(queuedPromptTitleLabel(1, false), "QUEUE • 1 prompt")
  assert.equal(queuedPromptActionLabel("steer", false), "steer")
  assert.equal(queuedPromptActionLabel("cancel", false), "cancel")
})

test("queued prompt strip metadata stays compact", () => {
  assert.equal(queuedPromptMetaLabel({ status: "Queued", attachmentCount: 0 }), "queued")
  assert.equal(queuedPromptMetaLabel({ status: "dispatching", attachmentCount: 1 }), "dispatching · 1 file")
  assert.equal(queuedPromptMetaLabel({ status: "queued", attachmentCount: 2 }), "queued · 2 files")
})
