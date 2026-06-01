import assert from "node:assert/strict"
import test from "node:test"

import type { SessionHistoryOutlineAgent, SessionHistoryPageEntry } from "./cli-types.js"
import { hydrateOutlineAgentEntries } from "./session-history-outline.js"

test("history outline expands provider auth notices by default", () => {
  const agent: SessionHistoryOutlineAgent = {
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      started_at_ms: 1,
      user_prompt: pageEntry(1, "user_prompt", "hi"),
      entries: [],
      blobs: [{
        blob_id: "blob-1",
        kind: "notice",
        title: "provider failed",
        summary: "401 Unauthorized: Missing bearer authentication",
        sequence_start: 2,
        sequence_end: 2,
        entry_count: 1,
        total_chars: 44,
        timestamp_ms: 2,
      }],
    }],
  }

  const entries = hydrateOutlineAgentEntries(agent)
  const notice = entries.find((entry) => entry.historyBlobId === "blob-1")

  assert.equal(notice?.blobCollapsed, false)
})

function pageEntry(
  entryIndex: number,
  kind: SessionHistoryPageEntry["entry"]["kind"],
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
