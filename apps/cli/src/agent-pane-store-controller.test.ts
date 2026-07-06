import assert from "node:assert/strict"
import test from "node:test"

import { createAgentPaneStoreController } from "./agent-pane-store-controller.js"
import type { AgentInstance, TranscriptEntry } from "./cli-types.js"

test("agent pane store persists visible transcript entries and preview", () => {
  let paneEntries: Record<string, TranscriptEntry[]> = {}
  let panePreviews: Record<string, string> = {}
  const sourceEntries = [entry(1, "user", "hello")]
  const controller = createAgentPaneStoreController({
    isAttached: () => true,
    getVisibleTranscriptAgentId: () => "agent-a",
    getVisibleTranscriptEntries: () => sourceEntries,
    getPaneEntriesByAgent: () => paneEntries,
    updatePaneEntries: (updater) => {
      paneEntries = updater(paneEntries)
    },
    updatePanePreviews: (updater) => {
      panePreviews = updater(panePreviews)
    },
    getSessionAgents: () => [agent("agent-a")],
    getFocusedAgentId: () => "agent-a",
    getMaxAgentsPerScreen: () => 2,
    splitAgentResponseMode: () => false,
    getPrimaryAgentId: () => "agent-a",
    expandedTurnIdsForAgent: () => [],
    replaceTranscriptEntries: () => {},
    reconcileMountedAuxiliaryTranscript: () => {},
  })

  controller.persistVisibleTranscriptEntries(sourceEntries)

  assert.deepEqual(paneEntries["agent-a"], sourceEntries)
  assert.notEqual(paneEntries["agent-a"], sourceEntries)
  assert.equal(panePreviews["agent-a"], "You: hello")
})

test("agent pane store mirrors primary and auxiliary split panes", () => {
  let paneEntries: Record<string, TranscriptEntry[]> = {
    "agent-b": [entry(1, "assistant", "old")],
  }
  const replaced: Array<{ agentId: string; entries: TranscriptEntry[] }> = []
  const reconciled: Array<{
    agentId: string
    previousEntries: TranscriptEntry[]
    nextEntries: TranscriptEntry[]
  }> = []
  const controller = createAgentPaneStoreController({
    isAttached: () => true,
    getVisibleTranscriptAgentId: () => "agent-a",
    getVisibleTranscriptEntries: () => [],
    getPaneEntriesByAgent: () => paneEntries,
    updatePaneEntries: (updater) => {
      paneEntries = updater(paneEntries)
    },
    updatePanePreviews: () => {},
    getSessionAgents: () => [agent("agent-a"), agent("agent-b")],
    getFocusedAgentId: () => "agent-a",
    getMaxAgentsPerScreen: () => 2,
    splitAgentResponseMode: () => true,
    getPrimaryAgentId: () => "agent-a",
    expandedTurnIdsForAgent: () => [],
    replaceTranscriptEntries: (entries, agentId) => {
      replaced.push({ agentId, entries })
    },
    reconcileMountedAuxiliaryTranscript: (agentId, previousEntries, nextEntries) => {
      reconciled.push({ agentId, previousEntries, nextEntries })
    },
  })

  controller.setAgentTranscriptEntries("agent-a", [entry(2, "assistant", "primary")])
  controller.setAgentTranscriptEntries("agent-b", [entry(3, "assistant", "side")])

  assert.deepEqual(replaced, [{
    agentId: "agent-a",
    entries: [{ ...entry(2, "assistant", "primary"), hidden: false }],
  }])
  assert.deepEqual(reconciled, [{
    agentId: "agent-b",
    previousEntries: [entry(1, "assistant", "old")],
    nextEntries: [{ ...entry(3, "assistant", "side"), hidden: false }],
  }])
})

test("agent pane store uses shared projection for collapsed turn state", () => {
  let paneEntries: Record<string, TranscriptEntry[]> = {}
  let panePreviews: Record<string, string> = {}
  const controller = createAgentPaneStoreController({
    isAttached: () => true,
    getVisibleTranscriptAgentId: () => "agent-a",
    getVisibleTranscriptEntries: () => [],
    getPaneEntriesByAgent: () => paneEntries,
    updatePaneEntries: (updater) => {
      paneEntries = updater(paneEntries)
    },
    updatePanePreviews: (updater) => {
      panePreviews = updater(panePreviews)
    },
    getSessionAgents: () => [agent("agent-a")],
    getFocusedAgentId: () => "agent-a",
    getMaxAgentsPerScreen: () => 2,
    splitAgentResponseMode: () => false,
    getPrimaryAgentId: () => "agent-a",
    expandedTurnIdsForAgent: () => [1],
    replaceTranscriptEntries: () => {},
    reconcileMountedAuxiliaryTranscript: () => {},
  })

  controller.setAgentTranscriptEntries("agent-a", [
    entry(1, "user", "prompt", { turnId: 1 }),
    entry(2, "reasoning", "thinking", { turnId: 1 }),
    entry(3, "assistant", "summary", { turnId: 1 }),
  ])

  assert.deepEqual(
    paneEntries["agent-a"]?.map((item) => [item.id, item.role, item.hidden ?? false, item.toggleMode ?? null]),
    [
      [1, "user", false, null],
      [4, "turn_toggle", false, "expand"],
      [2, "reasoning", true, null],
      [3, "assistant", false, null],
    ],
  )
  assert.equal(panePreviews["agent-a"], "You: prompt\nAsst: summary")
})

function entry(
  id: number,
  role: TranscriptEntry["role"],
  text: string,
  overrides: Partial<TranscriptEntry> = {},
): TranscriptEntry {
  return { id, role, text, ...overrides }
}

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/workspace/tree",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}
