import assert from "node:assert/strict"
import test from "node:test"

import type {
  SessionHistoryEntry,
  SessionHistoryOutlineBlob,
  SessionHistoryOutlineTurn,
  SessionHistoryPageEntry,
} from "./kernel-types.js"
import {
  orderedSessionHistoryOutlineItems,
  orderedSessionHistoryOutlineTurns,
  sessionHistoryCursorForVisibleAgent,
  sessionHistoryEntryKindTranscriptRole,
  sessionHistoryOutlineBlobSequenceStart,
  sessionHistoryOutlineTurnCompletedAtMs,
  sessionHistoryOutlineTurnDisplayId,
  sessionHistoryOutlineTurnKey,
  sessionHistoryOutlineTurnLifecycle,
  sessionHistoryOutlineTurnPromptMetadata,
  sessionHistoryOutlineTurnSourceAttachmentId,
  sessionHistoryPageEntryIndex,
} from "./session-history-outline.js"

test("session history outline turns are ordered by prompt history sequence", () => {
  const late = turn(20)
  const early = turn(10)

  assert.deepEqual(orderedSessionHistoryOutlineTurns([late, early]), [early, late])
})

test("session history outline turns break equal sequence ties deterministically", () => {
  const late = { ...turn(10), turn_id: "turn-b", started_at_ms: 20 }
  const early = { ...turn(10), turn_id: "turn-a", started_at_ms: 10 }
  const missingStart = { ...turn(10), turn_id: "turn-c", started_at_ms: Number.NaN }

  assert.deepEqual(orderedSessionHistoryOutlineTurns([missingStart, late, early]), [
    early,
    late,
    missingStart,
  ])
})

test("session history outline items are ordered by entry and blob sequence", () => {
  const items = orderedSessionHistoryOutlineItems({
    entries: [
      pageEntry(12, "provider_output", "reply"),
      pageEntry(10, "provider_reasoning", "thinking"),
    ],
    blobs: [
      blob(11, "provider_tool"),
    ],
    summary: pageEntry(13, "provider_output", "summary"),
  })

  assert.deepEqual(items.map((item) => item.kind === "entry" ? item.entry.entry.kind : item.blob.kind), [
    "provider_reasoning",
    "provider_tool",
    "provider_output",
    "provider_output",
  ])
})

test("session history outline items break equal sequence ties deterministically", () => {
  const items = orderedSessionHistoryOutlineItems({
    entries: [
      pageEntry(10, "provider_output", "reply-b", { timestamp_ms: 20 }),
      pageEntry(10, "provider_reasoning", "thinking-a", { timestamp_ms: 10 }),
    ],
    blobs: [
      { ...blob(10, "provider_tool"), blob_id: "blob-b", sequence_end: 12 },
      { ...blob(10, "provider_tool"), blob_id: "blob-a", sequence_end: 11 },
    ],
    summary: pageEntry(10, "provider_output", "summary", { timestamp_ms: 30 }),
  })

  assert.deepEqual(items.map((item) => item.kind === "entry" ? item.entry.entry.text : item.blob.blob_id), [
    "thinking-a",
    "reply-b",
    "summary",
    "blob-a",
    "blob-b",
  ])
})

test("session history outline sequence helpers use deterministic fallback values", () => {
  assert.equal(sessionHistoryPageEntryIndex(pageEntry(7, "user_prompt", "prompt")), 7)
  assert.equal(sessionHistoryOutlineBlobSequenceStart(blob(8, "provider_tool")), 8)
  assert.equal(sessionHistoryPageEntryIndex({ ...pageEntry(Number.NaN, "user_prompt", "prompt") }), Number.MAX_SAFE_INTEGER)
  assert.equal(sessionHistoryOutlineBlobSequenceStart({ ...blob(Number.NaN, "provider_tool") }), Number.MAX_SAFE_INTEGER)
})

test("session history outline turn display id follows durable prompt sequence", () => {
  assert.equal(sessionHistoryOutlineTurnDisplayId(turn(20), 0), 21)
  assert.equal(sessionHistoryOutlineTurnDisplayId(turn(Number.NaN), 4), 5)
})

test("session history outline turn key follows durable prompt identity and sequence", () => {
  assert.equal(sessionHistoryOutlineTurnKey("agent-1", {
    ...turn(7),
    prompt_id: "prompt-1",
  }), "agent-1:prompt-1:7")
  assert.equal(sessionHistoryOutlineTurnKey("agent-1", turn(7)), "agent-1:turn-7:7")
})

test("session history outline completion distinguishes absent, open, and settled markers", () => {
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs({}), undefined)
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs(turn(1)), null)
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs({ ...turn(1), completed_at_ms: null }), null)
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs({ ...turn(1), completed_at_ms: Number.NaN }), null)
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs({ ...turn(1), completed_at_ms: 123 }), 123)
})

test("session history outline lifecycle follows explicit protocol state", () => {
  assert.equal(sessionHistoryOutlineTurnLifecycle({ ...turn(1), lifecycle: "open" }), "open")
  assert.equal(sessionHistoryOutlineTurnLifecycle({ ...turn(1), lifecycle: "completed" }), "completed")
})

test("session history outline prompt metadata follows durable prompt entry identity", () => {
  const external = {
    ...turn(1),
    prompt_origin: " External ",
    user_prompt: pageEntry(1, "user_prompt", "prompt", {
      source_attachment_id: "attachment-1",
    }),
  } satisfies SessionHistoryOutlineTurn

  assert.equal(sessionHistoryOutlineTurnSourceAttachmentId(external), "attachment-1")
  assert.deepEqual(sessionHistoryOutlineTurnPromptMetadata(external), {
    promptOrigin: "external",
    sourceAttachmentId: "attachment-1",
  })
  assert.deepEqual(sessionHistoryOutlineTurnPromptMetadata({
    ...external,
    prompt_origin: null,
  }), {
    promptOrigin: null,
    sourceAttachmentId: "attachment-1",
  })
  assert.deepEqual(sessionHistoryOutlineTurnPromptMetadata({
    ...turn(3),
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "turn-1",
  }), {
    promptOrigin: "external",
  })
  assert.deepEqual(sessionHistoryOutlineTurnPromptMetadata({
    ...turn(4),
    prompt_id: "external:codex:thread-1:turn-1",
  }), {})
  assert.deepEqual(sessionHistoryOutlineTurnPromptMetadata({
    ...turn(5),
    prompt_id: "external:codex:thread-1:turn-1",
    prompt_origin: "arroba",
  }), {
    promptOrigin: "arroba",
  })
  assert.deepEqual(sessionHistoryOutlineTurnPromptMetadata(turn(2)), {})
  assert.equal(sessionHistoryOutlineTurnSourceAttachmentId({
    ...external,
    user_prompt: pageEntry(1, "user_prompt", "prompt", {
      source_attachment_id: null,
    }),
  }), null)
})

test("session history cursor selection follows the visible agent", () => {
  assert.deepEqual(sessionHistoryCursorForVisibleAgent({
    agents: [{
      agent_id: "agent-1",
      turns: [],
      next_cursor: { before_sequence: 10 },
    }, {
      agent_id: "agent-2",
      turns: [],
      next_cursor: null,
    }],
  }, "agent-1"), {
    agentId: "agent-1",
    cursor: { before_sequence: 10 },
  })

  assert.equal(sessionHistoryCursorForVisibleAgent({ agents: [] }, "agent-1"), null)
  assert.equal(sessionHistoryCursorForVisibleAgent({ agents: [] }, null), null)
  assert.equal(sessionHistoryCursorForVisibleAgent({
    agents: [{ agent_id: "agent-2", turns: [], next_cursor: null }],
  }, "agent-2"), null)
})

test("session history entry kind maps to transcript role", () => {
  assert.equal(sessionHistoryEntryKindTranscriptRole("user_prompt"), "user")
  assert.equal(sessionHistoryEntryKindTranscriptRole("provider_output"), "assistant")
  assert.equal(sessionHistoryEntryKindTranscriptRole("provider_reasoning"), "reasoning")
  assert.equal(sessionHistoryEntryKindTranscriptRole("provider_tool"), "tool")
  assert.equal(sessionHistoryEntryKindTranscriptRole("provider_error"), "error")
  assert.equal(sessionHistoryEntryKindTranscriptRole("provider_status"), "status")
  assert.equal(sessionHistoryEntryKindTranscriptRole("notice"), "notice")
})

function turn(promptEntryIndex: number): SessionHistoryOutlineTurn {
  return {
    turn_id: `turn-${promptEntryIndex}`,
    started_at_ms: promptEntryIndex,
    lifecycle: "open",
    completed_at_ms: null,
    user_prompt: pageEntry(promptEntryIndex, "user_prompt", "prompt"),
    entries: [],
    summary: null,
    blobs: [],
  }
}

function pageEntry(
  entryIndex: number,
  kind: SessionHistoryEntry["kind"],
  text: string,
  overrides: Partial<SessionHistoryEntry> = {},
): SessionHistoryPageEntry {
  return {
    entry_index: entryIndex,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text, ...overrides },
  }
}

function blob(
  sequenceStart: number,
  kind: SessionHistoryEntry["kind"],
): SessionHistoryOutlineBlob {
  return {
    blob_id: `blob-${sequenceStart}`,
    kind,
    title: kind,
    summary: kind,
    sequence_start: sequenceStart,
    sequence_end: sequenceStart,
    entry_count: 1,
    total_chars: 10,
    timestamp_ms: sequenceStart,
  }
}
