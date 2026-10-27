#!/usr/bin/env node
import assert from "node:assert/strict"

import {
  hydrateSessionHistoryOutlineAgentEntries,
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
} from "@chariox/kernel-client/session-history-transcript"

const outlineAgent = {
  agent_id: "agent-history-drill",
  turns: [{
    turn_id: "turn-1",
    prompt_id: "prompt-1",
    started_at_ms: 1,
    user_prompt: historyEntry(1, "user_prompt", "Summarize the fixture.", "agent-history-drill"),
    entries: [historyEntry(2, "provider_output", "Assistant detail before tool.", "agent-history-drill")],
    blobs: [{
      blob_id: "history:3:3",
      kind: "provider_tool",
      title: "bash · COMPLETED",
      summary: "bash completed",
      sequence_start: 3,
      sequence_end: 3,
      entry_count: 1,
      total_chars: 18,
      timestamp_ms: 3,
    }],
    summary: historyEntry(4, "provider_output", "Fixture summarized.", "agent-history-drill"),
  }],
  next_cursor: { before_sequence: 1 },
}

const entries = hydrateSessionHistoryOutlineAgentEntries(outlineAgent)
const placeholder = entries.find((entry) => entry.historyBlobId === "history:3:3")

assert.equal(entries.some((entry) => entry.role === "user" && entry.text === "Summarize the fixture."), true)
assert.equal(entries.some((entry) => entry.role === "assistant" && entry.text === "Assistant detail before tool."), true)
assert.equal(entries.some((entry) => entry.role === "assistant" && entry.text === "Fixture summarized."), true)
assert.equal(placeholder?.blobCollapsed, true)
assert.equal(placeholder?.historyBlobLoaded, false)

const loading = markSessionHistoryBlobLoading(entries, placeholder.id, true)
assert.equal(loading.find((entry) => entry.id === placeholder.id)?.historyBlobLoading, true)
assert.equal(loading.find((entry) => entry.id === placeholder.id)?.blobSummary, "loading...")

const expanded = replaceSessionHistoryBlobPlaceholder(entries, placeholder.id, {
  blob_id: "history:3:3",
  entries: [historyEntry(3, "provider_tool", "expanded tool body", "agent-history-drill")],
}, [1])

assert.equal(expanded.some((entry) => entry.historyBlobId === "history:3:3"), false)
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
