import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createTranscriptEntryProjectionController } from "./transcript-entry-projection-controller.js"

function entry(id: number, hidden = false): TranscriptEntry {
  return {
    id,
    role: "assistant",
    text: `entry-${id}`,
    hidden,
  }
}

test("transcript entry projection keeps renderable entries separate from visible entries", () => {
  const entries = [
    entry(1),
    null,
    entry(2, true),
    undefined,
    false as false,
    entry(3),
  ]
  const controller = createTranscriptEntryProjectionController({
    getEntries: () => entries,
  })

  assert.deepEqual(controller.renderableEntries().map((current) => current.id), [1, 2, 3])
  assert.deepEqual(controller.visibleEntries().map((current) => current.id), [1, 3])
  assert.equal(controller.visibleEntryCount(), 2)
})
