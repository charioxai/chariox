import assert from "node:assert/strict"
import test from "node:test"

import {
  applyTranscriptProviderChunk,
  applyTranscriptToolUpdate,
  computeCurrentTranscriptTurnId,
  computeNextTranscriptEntryId,
  normalizeTranscriptProviderChunk,
  transcriptStreamRuntimeOptions,
  transcriptStreamRuntimeTransition,
  type TranscriptStreamEntry,
} from "./transcript-stream-state.js"
import type { ToolTranscriptUpdate } from "@arroba/tool-display"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "./external-provider-observation.js"

test("transcript stream state appends provider chunks to the current turn", () => {
  const result = applyTranscriptProviderChunk([
    entry(1, "user", "prompt", { turnId: 4 }),
  ], {
    role: "assistant",
    chunk: "hello\r\n",
  })

  assert.equal(result.kind, "appended")
  assert.equal(result.updatedEntryId, 2)
  assert.deepEqual(result.entries, [
    entry(1, "user", "prompt", { turnId: 4 }),
    entry(2, "assistant", "hello\n", { turnId: 4 }),
  ])
})

test("transcript stream state can use an explicit current turn id before visible entries exist", () => {
  const result = applyTranscriptProviderChunk([], {
    role: "assistant",
    chunk: "hello",
    currentTurnId: 7,
  })

  assert.equal(result.kind, "appended")
  assert.deepEqual(result.entries, [
    entry(1, "assistant", "hello", { turnId: 7 }),
  ])
})

test("transcript stream state accepts a caller-owned next entry id", () => {
  const result = applyTranscriptProviderChunk([
    entry(2, "assistant", "visible retained entry"),
  ], {
    role: "assistant",
    chunk: "new",
    nextEntryId: 11,
  })

  assert.equal(result.kind, "merged")
  assert.equal(result.updatedEntryId, 2)

  const appended = applyTranscriptProviderChunk([
    entry(2, "assistant", "visible retained entry", { turnId: 1 }),
    entry(3, "user", "next turn", { turnId: 2 }),
  ], {
    role: "assistant",
    chunk: "new",
    nextEntryId: 11,
  })

  assert.equal(appended.kind, "appended")
  assert.equal(appended.updatedEntryId, 11)
  assert.equal(appended.entries.at(-1)?.id, 11)
})

test("transcript stream state scopes merge keys by provider run when supplied", () => {
  const result = applyTranscriptProviderChunk([
    entry(1, "assistant", "first", { mergeKey: "reply", providerRunId: "run-1" }),
  ], {
    role: "assistant",
    chunk: "second",
    mergeKey: "reply",
    providerRunId: "run-2",
  })

  assert.equal(result.kind, "appended")
  assert.deepEqual(result.entries, [
    entry(1, "assistant", "first", { mergeKey: "reply", providerRunId: "run-1" }),
    entry(2, "assistant", "second", { mergeKey: "reply", providerRunId: "run-2" }),
  ])
})

test("transcript stream state scopes merge keys by exact external observed identity", () => {
  const result = applyTranscriptProviderChunk([
    entry(1, "assistant", "first", {
      mergeKey: "reply",
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
    }),
  ], {
    role: "assistant",
    chunk: "second",
    mergeKey: "reply",
    metadata: {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-2",
    },
  })

  assert.equal(result.kind, "appended")
  assert.deepEqual(result.entries, [
    entry(1, "assistant", "first", {
      mergeKey: "reply",
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
    }),
    entry(2, "assistant", "second", {
      mergeKey: "reply",
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-2",
    }),
  ])
})

test("transcript stream state lets callers choose adjacent unkeyed merge roles", () => {
  const result = applyTranscriptProviderChunk([
    entry(1, "assistant", "first"),
  ], {
    role: "assistant",
    chunk: "second",
    mergeAdjacentUnkeyedRoles: ["reasoning"],
  })

  assert.equal(result.kind, "appended")
  assert.deepEqual(result.entries.map((candidate) => candidate.text), ["first", "second"])
})

test("transcript stream state merges adjacent assistant chunks", () => {
  const result = applyTranscriptProviderChunk([
    entry(1, "assistant", "hel"),
  ], {
    role: "assistant",
    chunk: "lo",
  })

  assert.equal(result.kind, "merged")
  assert.equal(result.updatedEntryId, 1)
  assert.equal(result.entries[0]?.text, "hello")
})

test("transcript stream state preserves prompt identity while merging chunks", () => {
  const first = applyTranscriptProviderChunk<TranscriptStreamEntry>([], {
    role: "assistant",
    chunk: "hel",
    mergeKey: "reply-1",
    metadata: {
      promptId: "prompt-1",
      promptOrigin: "external",
      sourceAttachmentId: "attachment-1",
    },
  })
  const second = applyTranscriptProviderChunk(first.entries, {
    role: "assistant",
    chunk: "lo",
    mergeKey: "reply-1",
    metadata: {
      promptId: "prompt-1",
      promptOrigin: "external",
      sourceAttachmentId: "attachment-1",
    },
  })

  assert.equal(second.kind, "merged")
  assert.equal(second.entries[0]?.text, "hello")
  assert.equal(second.entries[0]?.promptId, "prompt-1")
  assert.equal(second.entries[0]?.promptOrigin, "external")
  assert.equal(second.entries[0]?.sourceAttachmentId, "attachment-1")
})

test("transcript stream state scopes reused merge keys to the current turn", () => {
  const result = applyTranscriptProviderChunk([
    entry(1, "user", "first", { turnId: 1 }),
    entry(2, "assistant", "first reply", { turnId: 1, mergeKey: "reply" }),
    entry(3, "user", "second", { turnId: 2 }),
  ], {
    role: "assistant",
    chunk: "second reply",
    mergeKey: "reply",
  })

  assert.equal(result.kind, "appended")
  assert.deepEqual(result.entries, [
    entry(1, "user", "first", { turnId: 1 }),
    entry(2, "assistant", "first reply", { turnId: 1, mergeKey: "reply" }),
    entry(3, "user", "second", { turnId: 2 }),
    entry(4, "assistant", "second reply", { turnId: 2, mergeKey: "reply" }),
  ])
})

test("transcript stream state updates structured tool entries", () => {
  const toolState = new Map<string, ToolTranscriptUpdate>()
  const first = applyTranscriptToolUpdate<TranscriptStreamEntry>(
    [],
    '{"id":"tool-1","tool":"bash","status":"running"}',
    toolState,
  )
  const second = applyTranscriptToolUpdate(
    first.entries,
    '{"id":"tool-1","tool":"bash","status":"completed","output":"done"}',
    toolState,
  )

  assert.equal(toolState.get("tool-1")?.status, "completed")
  assert.equal(first.kind, "appended")
  assert.equal(second.kind, "merged")
  assert.equal(second.updatedEntryId, 1)
  assert.equal(second.entries[0]?.mergeKey, "tool-1")
  assert.equal(second.entries[0]?.sourceText, JSON.stringify(toolState.get("tool-1")))
})

test("transcript stream state keeps unstructured tool output as source text", () => {
  const result = applyTranscriptToolUpdate<TranscriptStreamEntry>([], "plain output", new Map())

  assert.equal(result.kind, "appended")
  assert.equal(result.entries[0]?.role, "tool")
  assert.equal(result.entries[0]?.text, "plain output")
  assert.equal(result.entries[0]?.sourceText, "plain output")
})

test("transcript stream helpers compute current turn and next ids", () => {
  assert.equal(normalizeTranscriptProviderChunk("a\r\nb\rc"), "a\nb\nc")
  assert.equal(computeCurrentTranscriptTurnId([
    entry(1, "user", "first", { turnId: 3 }),
    entry(2, "assistant", "reply", { turnId: 3 }),
    entry(3, "user", "second", { turnId: 4 }),
  ]), 4)
  assert.equal(computeNextTranscriptEntryId([
    entry(3, "assistant", "reply"),
    entry(7, "tool", "tool"),
  ]), 8)
})

test("transcript stream runtime options project caller counters", () => {
  assert.deepEqual(transcriptStreamRuntimeOptions({
    entryCounter: 9,
    currentTurnId: 4,
  }), {
    nextEntryId: 10,
    currentTurnId: 4,
  })
  assert.deepEqual(transcriptStreamRuntimeOptions({
    entryCounter: 0,
    currentTurnId: null,
  }), {
    nextEntryId: 1,
    currentTurnId: null,
  })
})

test("transcript stream state treats empty normalized chunks as no-op", () => {
  const result = applyTranscriptProviderChunk([entry(1, "assistant", "reply")], {
    role: "assistant",
    chunk: "",
  })

  assert.equal(result.kind, "noop")
  assert.deepEqual(result.entries, [entry(1, "assistant", "reply")])
})

test("transcript stream runtime transition marks only changed streams active", () => {
  assert.deepEqual(transcriptStreamRuntimeTransition({
    kind: "noop",
    entries: [],
  }), {
    shouldApplyRuntimeActivity: false,
    shouldCancelPendingTurnCompletion: false,
    working: null,
    submitting: null,
    shouldScheduleConfirmedTurnCompletion: false,
  })
  assert.deepEqual(transcriptStreamRuntimeTransition({
    kind: "appended",
    entries: [entry(1, "assistant", "reply")],
    updatedEntryId: 1,
  }), {
    shouldApplyRuntimeActivity: true,
    shouldCancelPendingTurnCompletion: true,
    working: true,
    submitting: false,
    shouldScheduleConfirmedTurnCompletion: true,
  })
  assert.deepEqual(transcriptStreamRuntimeTransition({
    kind: "merged",
    entries: [entry(1, "assistant", "reply")],
    updatedEntryId: 1,
  }), {
    shouldApplyRuntimeActivity: true,
    shouldCancelPendingTurnCompletion: true,
    working: true,
    submitting: false,
    shouldScheduleConfirmedTurnCompletion: true,
  })
})

function entry(
  id: number,
  role: string,
  text: string,
  overrides: Partial<TranscriptStreamEntry> = {},
): TranscriptStreamEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
