import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  hydrateTranscriptEntries,
  mergeAdjacentHistoryPageEntries,
  stitchPrependedHistory,
} from "./transcript-history.js"

test("hydrateTranscriptEntries reconstructs tool updates and suppresses idle status noise", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 0,
      fragment_start: 0,
      fragment_end: 6,
      total_chars: 6,
      entry: { kind: "user_prompt", text: "build\n" },
    },
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 15,
      total_chars: 15,
      entry: { kind: "provider_reasoning", text: "thinking...\n", merge_key: "r-1" },
    },
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 46,
      total_chars: 46,
      entry: {
        kind: "provider_tool",
        text: JSON.stringify({
          id: "tool-1",
          tool: "bash",
          status: "running",
          input: { command: "npm test" },
        }),
      },
    },
    {
      entry_index: 3,
      fragment_start: 0,
      fragment_end: 18,
      total_chars: 18,
      entry: { kind: "provider_status", text: "OpenCode is idle." },
    },
    {
      entry_index: 4,
      fragment_start: 0,
      fragment_end: 13,
      total_chars: 13,
      entry: { kind: "provider_output", text: "all green\n", merge_key: "a-1" },
    },
  ])

  assert.deepEqual(
    entries.map((entry) => entry.role),
    ["user", "reasoning", "tool", "assistant"],
  )
  assert.equal(entries[0]?.text, "build")
  assert.match(entries[2]?.text ?? "", /\*\*bash\*\*/)
  assert.match(entries[2]?.text ?? "", /npm test/)
  assert.equal(entries[3]?.text, "all green\n")
})

test("hydrateTranscriptEntries marks only the head partial fragment as deferred after rejoin catch-up", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 5,
      fragment_start: 120,
      fragment_end: 240,
      total_chars: 240,
      entry: { kind: "provider_output", text: "continued reply\n", merge_key: "reply-1" },
    },
    {
      entry_index: 6,
      fragment_start: 0,
      fragment_end: 12,
      total_chars: 12,
      entry: { kind: "notice", text: "reattached" },
    },
  ])

  assert.equal(entries[0]?.historyDeferred, true)
  assert.equal(entries[1]?.historyDeferred, undefined)
})

test("mergeAdjacentHistoryPageEntries preserves merge keys across stitched fragments", () => {
  const merged = mergeAdjacentHistoryPageEntries([
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 5,
      total_chars: 10,
      entry: { kind: "provider_output", text: "hello", merge_key: "a-1" },
    },
    {
      entry_index: 2,
      fragment_start: 5,
      fragment_end: 10,
      total_chars: 10,
      entry: { kind: "provider_output", text: " world", merge_key: "a-1" },
    },
  ])

  assert.equal(merged.length, 1)
  assert.equal(merged[0]?.entry.text, "hello world")
  assert.equal(merged[0]?.entry.merge_key, "a-1")
})

test("stitchPrependedHistory merges adjacent assistant fragments", () => {
  const stitched = stitchPrependedHistory(
    [entry(1, "assistant", "hello ", {
      historyEntryIndex: 7,
      historyFragmentStart: 0,
      historyFragmentEnd: 6,
      historyTotalChars: 11,
    })],
    [entry(2, "assistant", "world", {
      historyEntryIndex: 7,
      historyFragmentStart: 6,
      historyFragmentEnd: 11,
      historyTotalChars: 11,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.text, "hello world")
  assert.equal(stitched[0]?.sourceText, "hello world")
  assert.equal(stitched[0]?.historyFragmentStart, 0)
  assert.equal(stitched[0]?.historyFragmentEnd, 11)
  assert.equal(stitched[0]?.historyDeferred, undefined)
})

test("stitchPrependedHistory rebuilds structured tool fragments", () => {
  const toolPayload = JSON.stringify({
    id: "tool-1",
    tool: "bash",
    status: "completed",
    input: { command: "pnpm test" },
    output: "ok",
  })
  const splitAt = Math.floor(toolPayload.length / 2)

  const stitched = stitchPrependedHistory(
    [entry(1, "tool", toolPayload.slice(0, splitAt), {
      sourceText: toolPayload.slice(0, splitAt),
      historyEntryIndex: 9,
      historyFragmentStart: 0,
      historyFragmentEnd: splitAt,
      historyTotalChars: toolPayload.length,
      mergeKey: "stale",
    })],
    [entry(2, "tool", toolPayload.slice(splitAt), {
      sourceText: toolPayload.slice(splitAt),
      historyEntryIndex: 9,
      historyFragmentStart: splitAt,
      historyFragmentEnd: toolPayload.length,
      historyTotalChars: toolPayload.length,
      mergeKey: "stale",
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.mergeKey, "tool-1")
  assert.match(stitched[0]?.text ?? "", /\*\*bash\*\*/)
  assert.match(stitched[0]?.text ?? "", /pnpm test/)
  assert.equal(stitched[0]?.sourceText, toolPayload)
})

function entry(
  id: number,
  role: TranscriptEntry["role"],
  text: string,
  overrides: Partial<TranscriptEntry> = {},
): TranscriptEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
