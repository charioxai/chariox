import assert from "node:assert/strict"
import test from "node:test"

import {
  computeCurrentTranscriptTurnId,
  computeMaxTranscriptEntryId,
  computeNextTranscriptEntryId,
  computeNextTranscriptTurnId,
  createTranscriptSteeredPromptEntry,
  createTranscriptUserPromptTurn,
  createNextTranscriptEntry,
  reindexTranscriptEntries,
  shouldSkipConsecutiveTranscriptEntry,
  transcriptEntryRuntimeOptions,
  transcriptHasTrailingUserPrompt,
  transcriptRetentionSlice,
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
  assert.equal(computeMaxTranscriptEntryId([
    entry(8, "user", "first"),
    entry(14, "assistant", "reply"),
  ]), 14)
  assert.equal(computeMaxTranscriptEntryId([]), 0)
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

test("createTranscriptUserPromptTurn projects prompt entry and turn identity", () => {
  assert.deepEqual(createTranscriptUserPromptTurn("hello\n", 9), {
    entry: {
      role: "user",
      text: "hello",
      turnId: 9,
    },
    currentTurnId: 9,
    nextTurnId: 10,
  })
})

test("createTranscriptSteeredPromptEntry keeps steering out of turn tracking", () => {
  assert.deepEqual(createTranscriptSteeredPromptEntry("steer\n", {
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  }), {
    role: "user",
    text: "steer",
    turnTracking: "none",
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  })
  assert.equal(createTranscriptSteeredPromptEntry("\n"), null)
})

test("shouldSkipConsecutiveTranscriptEntry only deduplicates consecutive notices and errors", () => {
  assert.equal(
    shouldSkipConsecutiveTranscriptEntry(
      { role: "notice", text: "same", emphasis: "warning" },
      { role: "notice", text: "same", emphasis: "warning" },
    ),
    true,
  )
  assert.equal(
    shouldSkipConsecutiveTranscriptEntry(
      { role: "error", text: "same", emphasis: "error" },
      { role: "error", text: "same", emphasis: "error" },
    ),
    true,
  )
  assert.equal(
    shouldSkipConsecutiveTranscriptEntry(
      { role: "notice", text: "same", emphasis: "warning" },
      { role: "notice", text: "different", emphasis: "warning" },
    ),
    false,
  )
  assert.equal(
    shouldSkipConsecutiveTranscriptEntry(
      { role: "assistant", text: "same" },
      { role: "assistant", text: "same" },
    ),
    false,
  )
})

test("transcriptEntryRuntimeOptions projects caller counters", () => {
  assert.deepEqual(transcriptEntryRuntimeOptions({
    entryCounter: 9,
    currentTurnId: 4,
  }), {
    nextEntryId: 10,
    currentTurnId: 4,
  })
  assert.deepEqual(transcriptEntryRuntimeOptions({
    entryCounter: 0,
    currentTurnId: null,
  }), {
    nextEntryId: 1,
    currentTurnId: null,
  })
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

test("createNextTranscriptEntry accepts explicit runtime entry and turn identity", () => {
  const next = createNextTranscriptEntry([
    entry(4, "user", "previous", { turnId: 1 }),
  ], {
    role: "assistant",
    text: "reply",
  }, {
    nextEntryId: 12,
    currentTurnId: 7,
  })

  assert.deepEqual(next, entry(12, "assistant", "reply", { turnId: 7 }))
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

test("transcriptRetentionSlice trims old entries by count", () => {
  const entries = [
    entry(1, "assistant", "one"),
    entry(2, "assistant", "two"),
    entry(3, "assistant", "three"),
  ]

  const slice = transcriptRetentionSlice(entries, { maxEntries: 2, maxChars: 1_000 })

  assert.equal(slice.changed, true)
  assert.deepEqual(slice.removed.map((item) => item.id), [1])
  assert.deepEqual(slice.kept.map((item) => item.id), [2, 3])
  assert.deepEqual(entries.map((item) => item.id), [1, 2, 3])
})

test("transcriptRetentionSlice trims by characters while keeping the latest entry", () => {
  const slice = transcriptRetentionSlice([
    entry(1, "assistant", "older"),
    entry(2, "assistant", "large-current-entry"),
  ], { maxEntries: 10, maxChars: 4 })

  assert.equal(slice.changed, true)
  assert.deepEqual(slice.removed.map((item) => item.id), [1])
  assert.deepEqual(slice.kept.map((item) => item.id), [2])
})

test("transcriptRetentionSlice reports unchanged entries", () => {
  const slice = transcriptRetentionSlice([
    entry(1, "assistant", "one"),
  ], { maxEntries: 2, maxChars: 10 })

  assert.equal(slice.changed, false)
  assert.deepEqual(slice.removed, [])
  assert.deepEqual(slice.kept.map((item) => item.id), [1])
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
