import assert from "node:assert/strict"
import test from "node:test"

import { hydrateTranscriptEntries, mergeAdjacentHistoryPageEntries } from "./transcript-history.js"

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
