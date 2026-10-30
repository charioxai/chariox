import test from "node:test"
import assert from "node:assert/strict"
import type { SessionHistoryPageEntry, SessionHistoryOutlineTurn } from "@chariox/kernel-client/kernel-types"
import { renderTerminalEntriesByTurn, type TerminalTurnState } from "@chariox/transcript-view"
import { AgentTriggerClient, agentTriggerHistoryEntries } from "./agent-trigger-client.js"
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
test("MP-08/MP-10/MP-11: a final answer past the inline limit is rebuilt from its blob rows", () => {
  const first = "a".repeat(10_000), second = "b".repeat(10_000)
  const inline = (first + second).slice(0, 16_384)
  const turn: SessionHistoryOutlineTurn = { turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 9, lifecycle: "completed",
    user_prompt: row(1, "user_prompt", "Question"), entries: [],
    summary: { ...row(2, "provider_output", inline), fragment_end: inline.length, total_chars: 20_000 },
    blobs: [{ blob_id: "2:3", title: "assistant output", summary: "", preview: null } as unknown as SessionHistoryOutlineTurn["blobs"][number]] }
  const entries = agentTriggerHistoryEntries("agent", [turn], new Map([["2:3", [row(2, "provider_output", first), row(3, "provider_output", second)]]]))
  assert.deepEqual(entries.map(e => [e.key, e.role]), [["1", "user"], ["2", "assistant"]])
  assert.equal(entries[1]!.text, first + second)
  assert.equal(entries[1]!.historyTurnSummary, true)
})
test("MP-08/MP-10/MP-11: commentary before a tool call stays beside the final answer", () => {
  const turn: SessionHistoryOutlineTurn = { turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 9, lifecycle: "completed",
    user_prompt: row(1, "user_prompt", "Question"), entries: [row(2, "provider_output", "Let me look."), row(3, "provider_output", "(tool)")],
    summary: row(4, "provider_output", "Done."), blobs: [] }
  const entries = agentTriggerHistoryEntries("agent", [turn], new Map())
  assert.deepEqual(entries.map(e => e.text), ["Question", "Let me look.", "(tool)", "Done."])
  assert.deepEqual(entries.filter(e => e.historyTurnSummary).map(e => e.text), ["Done."])
})
test("MP-08/MP-10/MP-11: a page that cannot undo or fork a turn renders no Undo or Fork", () => {
  const entries = agentTriggerHistoryEntries("agent", [{ turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 2, lifecycle: "completed",
    user_prompt: row(1, "user_prompt", "Question"), entries: [], summary: row(2, "provider_output", "Answer"), blobs: [] }], new Map())
  const turn: TerminalTurnState = { id: "turn-1", agentId: "agent", startedAtMs: 1, lastEntryAtMs: 2, completedAtMs: 2, kernelObservedActive: false, kernelSettled: true, kernelSettlementStatus: "completed", expanded: false }
  const options = { turns: new Map([[turn.id, turn]]), escapeHtml: (v: string) => v, renderTranscriptHtml: (v: string) => v, renderEntry: (e: { text: string }) => e.text }
  assert.match(renderTerminalEntriesByTurn(entries, options), /data-freeform-turn-action="fork"/)
  assert.doesNotMatch(renderTerminalEntriesByTurn(entries, { ...options, turnActions: false }), /data-freeform-turn-action/)
})
test("MP-08/MP-10: a trigger prompt in history keeps the origin the kernel labelled it with", () => {
  const prompt = row(1, "user_prompt", "Check the build.")
  prompt.entry.prompt_origin = "schedule"
  const entries = agentTriggerHistoryEntries("agent", [{ turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 2, lifecycle: "completed",
    user_prompt: prompt, entries: [], summary: row(2, "provider_output", "Done."), blobs: [] }], new Map())
  assert.deepEqual(entries.map(entry => entry.promptOrigin ?? null), ["schedule", null])
})
test("MP-08/MP-10: a prompt past the inline limit is shown whole, read from its own history blob", async () => {
  const text = "p".repeat(20_000)
  const cut = text.slice(0, 16_384)
  const prompt = { ...row(1, "user_prompt", cut), fragment_end: cut.length, total_chars: text.length }
  const turn: SessionHistoryOutlineTurn = { turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 3, lifecycle: "completed",
    user_prompt: prompt, entries: [], summary: row(2, "provider_output", "Done."), blobs: [] }
  const requested: string[][] = []
  const client = new AgentTriggerClient("session", "trigger", "ignored")
  Object.assign(client, { kernel: { send: async (request: Record<string, any>) => {
    if (request.GetSessionHistoryBlobContents) {
      requested.push(request.GetSessionHistoryBlobContents.blob_ids)
      return { SessionHistoryBlobContents: { blobs: [{ blob_id: "history:1:1", entries: [row(1, "user_prompt", text)] }] } }
    }
    return { AgentTriggerTranscript: { trigger: { agent_id: "agent", enabled: true }, agent_alias: "a", online: true, busy: false, deliveries: [],
      history: { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [turn], next_cursor: null }] } } } }
  } } })
  const conversation = await client.conversation()
  assert.deepEqual(requested, [["history:1:1"]])
  assert.equal(conversation.entries[0]!.text, text)
})
