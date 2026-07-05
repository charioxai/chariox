import assert from "node:assert/strict"
import test from "node:test"

import {
  computeCurrentTranscriptTurnId,
  computeNextTranscriptEntryId,
  computeNextTranscriptTurnId,
  createNextTranscriptEntry,
  reindexTranscriptEntries,
  transcriptHasTrailingUserPrompt,
  trimSingleTrailingNewline,
  type TranscriptEntryStateEntry,
} from "./transcript-entry-state.js"

test("trimSingleTrailingNewline removes only one final newline", () => {
  assert.equal(trimSingleTrailingNewline("hello\n"), "hello")
  assert.equal(trimSingleTrailingNewline("hello\n\n"), "hello\n")
  assert.equal(trimSingleTrailingNewline("hello"), "hello")
})

test("reindexTranscriptEntries assigns ids after the starting id without mutating entries", () => {
  const entries = [
    entry(99, "user", "one"),
    entry(100, "assistant", "two"),
  ]
  const reindexed = reindexTranscriptEntries(entries, 10)

  assert.deepEqual(reindexed.map((item) => item.id), [11, 12])
  assert.deepEqual(entries.map((item) => item.id), [99, 100])
})

test("transcript turn id helpers project current and next turn identity", () => {
  assert.equal(computeCurrentTranscriptTurnId([
    entry(1, "user", "first", { turnId: 3 }),
    entry(2, "assistant", "reply", { turnId: 3 }),
    entry(3, "user", "second", { turnId: 7 }),
  ]), 7)
  assert.equal(computeCurrentTranscriptTurnId([
    entry(1, "assistant", "reply", { turnId: 3 }),
  ]), null)
  assert.equal(computeNextTranscriptTurnId([
    entry(1, "user", "first", { turnId: 3 }),
    entry(2, "assistant", "reply", { turnId: 9 }),
  ]), 10)
  assert.equal(computeNextTranscriptEntryId([
    entry(8, "user", "first"),
    entry(14, "assistant", "reply"),
  ]), 15)
})

test("transcriptHasTrailingUserPrompt dedupes prompt echoes by prompt id before text", () => {
  const entries = [
    entry(1, "user", "hello\n", { promptId: "prompt-1" }),
  ]

  assert.equal(transcriptHasTrailingUserPrompt(entries, "hello"), true)
  assert.equal(transcriptHasTrailingUserPrompt(entries, "changed display text", "prompt-1"), true)
  assert.equal(transcriptHasTrailingUserPrompt(entries, "hello\n", "prompt-2"), false)
  assert.equal(transcriptHasTrailingUserPrompt(entries, "other"), false)
  assert.equal(transcriptHasTrailingUserPrompt([
    entry(1, "assistant", "hello"),
  ], "hello"), false)
})

test("createNextTranscriptEntry assigns ids and inherits the active turn", () => {
  const next = createNextTranscriptEntry([
    entry(4, "user", "prompt", { turnId: 9 }),
    entry(8, "assistant", "working", { turnId: 9 }),
  ], {
    role: "tool",
    text: "tool output",
  })

  assert.deepEqual(next, entry(9, "tool", "tool output", { turnId: 9 }))
})

test("createNextTranscriptEntry preserves explicit turn ids", () => {
  const next = createNextTranscriptEntry([
    entry(4, "user", "prompt", { turnId: 9 }),
  ], {
    role: "assistant",
    text: "reply",
    turnId: 12,
  })

  assert.deepEqual(next, entry(5, "assistant", "reply", { turnId: 12 }))
})

function entry(
  id: number,
  role: string,
  text: string,
  overrides: Partial<TranscriptEntryStateEntry> = {},
): TranscriptEntryStateEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
