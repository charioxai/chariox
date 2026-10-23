import test from "node:test"
import assert from "node:assert/strict"
import type { SessionHistoryPageEntry, SessionHistoryOutlineTurn } from "@chariox/kernel-client/kernel-types"
import { agentTriggerHistoryEntries } from "./agent-trigger-client.js"
function row(entry_index: number, kind: "user_prompt" | "provider_output", text: string): SessionHistoryPageEntry {
  return { entry_index, fragment_start: 0, fragment_end: text.length, total_chars: text.length, entry: { kind, text, timestamp_ms: entry_index } }
}
test("MP-08/MP-10/MP-11: pre-trigger prompts and summaries retain distinct canonical entry identities", () => {
  const turns: SessionHistoryOutlineTurn[] = [1, 3].map(n => ({ turn_id: `turn-${n}`, prompt_id: `prompt-${n}`, started_at_ms: n,
    completed_at_ms: n + 1, lifecycle: "completed", user_prompt: row(n, "user_prompt", `Question ${n}`),
    entries: [], summary: row(n + 1, "provider_output", `Answer ${n}`), blobs: [] }))
  const entries = agentTriggerHistoryEntries("agent", turns, new Map())
  assert.deepEqual(entries.map(e => [e.key, e.role, e.text]), [["1", "user", "Question 1"], ["2", "assistant", "Answer 1"], ["3", "user", "Question 3"], ["4", "assistant", "Answer 3"]])
  assert.equal(entries.filter(e => e.historyTurnSummary).length, 2)
})
