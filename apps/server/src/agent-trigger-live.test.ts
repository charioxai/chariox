import test from "node:test"
import assert from "node:assert/strict"
import type { TerminalTurnState } from "@chariox/transcript-view"
import type { Conversation } from "./agent-trigger-client.js"
import { applyOutputRecords, reconcileSnapshot, type LiveState } from "./agent-trigger-live.js"
const empty = (): LiveState => ({ entries: new Map(), turns: new Map(), decoders: new Map() })
const bytes = (text: string) => [...new TextEncoder().encode(text)]
const openTurn: TerminalTurnState = { id: "turn-1", agentId: "agent", startedAtMs: 1, lastEntryAtMs: 1, kernelObservedActive: true, historyObserved: true, kernelPromptId: "active-1", expanded: false }
function snapshot(turnState: Partial<TerminalTurnState>, steered = true): Conversation {
  return { alias: "agent", online: true, busy: false, enabled: true, nextCursor: null, unconfirmed: [],
    turns: [{ ...openTurn, ...turnState }],
    entries: !steered ? [] : [{ key: "2", role: "user", agentId: "agent", turnId: "turn-1", createdAtMs: 2, text: "steer me", promptId: "active-1" }] }
}
test("MP-08/MP-09: a steering echo joins the running turn and the next snapshot replaces it", () => {
  const state = empty()
  reconcileSnapshot(state, snapshot({}, false))
  applyOutputRecords(state, [{ kind: "prompt_echo", agent_id: "agent", prompt_id: "steer-1", merge_key: "steering-prompt:steer-1", timestamp_ms: 2, bytes: bytes("steer me\n") }])
  assert.deepEqual([...state.turns.keys()], ["turn-1"], "no synthetic turn for the steering prompt")
  assert.deepEqual([...state.entries.values()].map(entry => [entry.turnId, entry.text]), [["turn-1", "steer me\n"]])
  reconcileSnapshot(state, snapshot({ kernelSettled: true, kernelObservedActive: false, completedAtMs: 5, kernelSettlementStatus: "completed" }))
  assert.deepEqual([...state.turns.values()].map(turn => [turn.id, turn.kernelSettled]), [["turn-1", true]])
  assert.deepEqual([...state.entries.values()].map(entry => entry.text), ["steer me"], "the steering message appears once")
  assert.equal(state.decoders.size, 0)
})
test("MP-08/MP-09: provider output for an unknown prompt still opens a live turn that history later replaces", () => {
  const state = empty()
  applyOutputRecords(state, [{ kind: "provider_output", agent_id: "agent", prompt_id: "active-1", merge_key: "m1", timestamp_ms: 3, bytes: bytes("hel") },
    { kind: "provider_output", agent_id: "agent", prompt_id: "active-1", merge_key: "m1", timestamp_ms: 3, bytes: bytes("lo") }])
  assert.deepEqual([...state.entries.values()].map(entry => entry.text), ["hello"])
  assert.deepEqual([...state.turns.keys()], ["live:active-1"])
  reconcileSnapshot(state, snapshot({}))
  assert.deepEqual([...state.turns.keys()], ["turn-1"])
  assert.deepEqual([...state.entries.values()].map(entry => entry.text), ["steer me"])
})
test("MP-08/MP-09: a status record without a prompt opens no turn, so a steering echo still joins the running one", () => {
  const state = empty()
  applyOutputRecords(state, [{ kind: "provider_status", agent_id: "agent", timestamp_ms: 9, bytes: bytes("external:codex") }])
  assert.equal(state.turns.size, 0)
  reconcileSnapshot(state, snapshot({}, false))
  applyOutputRecords(state, [{ kind: "provider_status", agent_id: "agent", timestamp_ms: 9, bytes: bytes("external:codex") },
    { kind: "prompt_echo", agent_id: "agent", prompt_id: "steer-1", merge_key: "steering-prompt:steer-1", timestamp_ms: 10, bytes: bytes("steer me\n") }])
  assert.deepEqual([...state.turns.keys()], ["turn-1"])
  assert.deepEqual([...state.entries.values()].map(entry => entry.turnId), ["turn-1"])
})
test("MP-08/MP-09: a steered trigger prompt keeps the origin the kernel labelled it with", () => {
  const state = empty()
  reconcileSnapshot(state, snapshot({}, false))
  applyOutputRecords(state, [{ kind: "prompt_echo", agent_id: "agent", prompt_id: "steer-1", prompt_origin: "http", merge_key: "steering-prompt:steer-1", timestamp_ms: 2, bytes: bytes("steer me\n") }])
  assert.deepEqual([...state.entries.values()].map(entry => entry.promptOrigin), ["http"])
})
