import test from "node:test"
import assert from "node:assert/strict"

import {
  reindexTranscriptEntries,
  trimSingleTrailingNewline,
} from "./transcript-text.js"

test("trimSingleTrailingNewline removes only one final newline", () => {
  assert.equal(trimSingleTrailingNewline("hello\n"), "hello")
  assert.equal(trimSingleTrailingNewline("hello\n\n"), "hello\n")
  assert.equal(trimSingleTrailingNewline("hello"), "hello")
})

test("reindexTranscriptEntries assigns ids after the starting id", () => {
  assert.deepEqual(
    reindexTranscriptEntries([
      { role: "user", text: "one", id: 99 },
      { role: "assistant", text: "two", id: 100 },
    ], 10).map((entry) => entry.id),
    [11, 12],
  )
})
