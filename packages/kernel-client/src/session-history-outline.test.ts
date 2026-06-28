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
  sessionHistoryOutlineBlobSequenceStart,
  sessionHistoryOutlineTurnCompletedAtMs,
  sessionHistoryOutlineTurnDisplayId,
  sessionHistoryPageEntryIndex,
} from "./session-history-outline.js"

test("session history outline turns are ordered by prompt history sequence", () => {
  const late = turn(20)
  const early = turn(10)

  assert.deepEqual(orderedSessionHistoryOutlineTurns([late, early]), [early, late])
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

test("session history outline completion distinguishes absent, open, and settled markers", () => {
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs(turn(1)), undefined)
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs({ ...turn(1), completed_at_ms: null }), null)
  assert.equal(sessionHistoryOutlineTurnCompletedAtMs({ ...turn(1), completed_at_ms: 123 }), 123)
})

function turn(promptEntryIndex: number): SessionHistoryOutlineTurn {
  return {
    turn_id: `turn-${promptEntryIndex}`,
    started_at_ms: promptEntryIndex,
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
): SessionHistoryPageEntry {
  return {
    entry_index: entryIndex,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text },
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
