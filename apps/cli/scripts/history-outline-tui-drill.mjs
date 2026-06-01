#!/usr/bin/env node
import assert from "node:assert/strict"

import {
  hydrateOutlineAgentEntries,
  markHistoryBlobLoading,
  replaceHistoryBlobPlaceholder,
} from "../dist/session-history-outline.js"

const outlineAgent = {
  agent_id: "agent-history-drill",
  turns: [{
    turn_id: "turn-1",
    prompt_id: "prompt-1",
    started_at_ms: 1,
    user_prompt: historyEntry(1, "user_prompt", "Summarize the fixture.", "agent-history-drill"),
    blobs: [{
      blob_id: "history:2:2",
      kind: "provider_tool",
      title: "bash · COMPLETED",
      summary: "bash completed",
      sequence_start: 2,
      sequence_end: 2,
      entry_count: 1,
      total_chars: 18,
      timestamp_ms: 2,
    }],
    summary: historyEntry(3, "provider_output", "Fixture summarized.", "agent-history-drill"),
  }],
  next_cursor: { before_sequence: 1 },
}

const entries = hydrateOutlineAgentEntries(outlineAgent)
const placeholder = entries.find((entry) => entry.historyBlobId === "history:2:2")

assert.equal(entries.some((entry) => entry.role === "user" && entry.text === "Summarize the fixture."), true)
assert.equal(entries.some((entry) => entry.role === "assistant" && entry.text === "Fixture summarized."), true)
assert.equal(placeholder?.blobCollapsed, true)
assert.equal(placeholder?.historyBlobLoaded, false)

const loading = markHistoryBlobLoading(entries, placeholder.id, true)
assert.equal(loading.find((entry) => entry.id === placeholder.id)?.historyBlobLoading, true)
assert.equal(loading.find((entry) => entry.id === placeholder.id)?.blobSummary, "loading...")

const expanded = replaceHistoryBlobPlaceholder(entries, placeholder.id, {
  blob_id: "history:2:2",
  entries: [historyEntry(2, "provider_tool", "expanded tool body", "agent-history-drill")],
}, [1])

assert.equal(expanded.some((entry) => entry.historyBlobId === "history:2:2"), false)
assert.equal(expanded.some((entry) => entry.role === "tool" && entry.text === "expanded tool body"), true)

console.log(JSON.stringify({
  drill: "history-outline-tui",
  outlineEntries: entries.length,
  placeholderId: placeholder.id,
  expandedEntries: expanded.length,
}, null, 2))

function historyEntry(index, kind, text, agentId) {
  return {
    entry_index: index,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: {
      kind,
      text,
      agent_id: agentId,
    },
  }
}
