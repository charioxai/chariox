import assert from "node:assert/strict"
import test from "node:test"

import type { SessionHistoryOutlineAgent } from "./cli-types.js"
import {
  hydrateOutlineAgentEntries,
  replaceHistoryBlobPlaceholder,
} from "./session-history-outline.js"

test("hydrateOutlineAgentEntries carries turn prompt identity into entries and blob placeholders", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "build\n"),
      entries: [pageEntry(1, "provider_reasoning", "thinking\n")],
      summary: pageEntry(2, "provider_output", "done\n"),
      blobs: [{
        blob_id: "blob-1",
        kind: "provider_tool",
        title: "tool",
        summary: "1 tool called",
        sequence_start: 3,
        sequence_end: 4,
        entry_count: 1,
        total_chars: 80,
        timestamp_ms: 1,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  assert.equal(entries.find((entry) => entry.role === "user")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.role === "reasoning")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.historyBlobId === "blob-1")?.promptId, "prompt-1")
})

test("replaceHistoryBlobPlaceholder keeps prompt identity when expanding blob content", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "build\n"),
      entries: [],
      summary: pageEntry(2, "provider_output", "done\n"),
      blobs: [{
        blob_id: "blob-1",
        kind: "provider_tool",
        title: "tool",
        summary: "1 tool called",
        sequence_start: 1,
        sequence_end: 2,
        entry_count: 1,
        total_chars: 80,
        timestamp_ms: 1,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.ok(placeholder)

  const replaced = replaceHistoryBlobPlaceholder(
    entries,
    placeholder.id,
    {
      blob_id: "blob-1",
      entries: [pageEntry(1, "provider_tool", JSON.stringify({
        id: "tool-1",
        tool: "bash",
        status: "completed",
        output: "ok",
      }))],
    },
    [],
  )

  assert.equal(replaced.find((entry) => entry.role === "tool")?.promptId, "prompt-1")
})

function pageEntry(
  entryIndex: number,
  kind: "user_prompt" | "provider_output" | "provider_reasoning" | "provider_tool",
  text: string,
) {
  return {
    entry_index: entryIndex,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text },
  }
}
