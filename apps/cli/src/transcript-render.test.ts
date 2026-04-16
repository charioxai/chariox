import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  TRANSCRIPT_ENTRY_PADDING,
  transcriptEntryPadding,
} from "./transcript-entry-style.js"

function entry(role: TranscriptEntry["role"]): TranscriptEntry {
  return {
    id: 1,
    role,
    text: "text",
  }
}

test("transcriptEntryPadding removes padding only from turn toggles", () => {
  assert.deepEqual(transcriptEntryPadding(entry("turn_toggle")), {
    horizontal: 0,
    vertical: 0,
  })

  assert.equal(transcriptEntryPadding(entry("assistant")), TRANSCRIPT_ENTRY_PADDING)
  assert.equal(transcriptEntryPadding({ ...entry("tool"), blobCollapsible: true }), TRANSCRIPT_ENTRY_PADDING)
  assert.equal(transcriptEntryPadding(entry("user")), TRANSCRIPT_ENTRY_PADDING)
})
