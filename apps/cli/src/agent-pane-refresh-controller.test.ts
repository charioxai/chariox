import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeSession,
  SessionHistoryCursor,
  SessionHistoryPageEntry,
} from "./cli-types.js"
import { createAgentPaneRefreshController } from "./agent-pane-refresh-controller.js"

test("agent pane refresh loads agent histories and replaces the visible split transcript", async () => {
  const harness = createHarness({ split: true })

  await harness.controller.refresh(session("a"))

  assert.deepEqual(harness.loads, ["a:null", "b:null"])
  assert.deepEqual(harness.replaced, { agentId: "a", text: ["hello"] })
  assert.deepEqual(harness.rebuiltAuxiliaryAgentIds, ["b"])
  assert.deepEqual(harness.previews, {
    a: "You: hello",
    b: "Asst: world",
  })
  assert.deepEqual(harness.calls.slice(-2), ["applyResponseLayout", "rebuildAuxiliaryAgentPane:b"])
})

test("agent pane refresh does not rebuild auxiliary panes outside split mode", async () => {
  const harness = createHarness({ split: false })

  await harness.controller.refresh(session("b"))

  assert.deepEqual(harness.loads, ["a:null", "b:null"])
  assert.deepEqual(harness.replaced, { agentId: "b", text: ["world\n"] })
  assert.deepEqual(harness.rebuiltAuxiliaryAgentIds, [])
  assert.deepEqual(harness.calls.slice(-1), ["applyResponseLayout"])
})

function createHarness(options: { split: boolean }) {
  const calls: string[] = []
  const loads: string[] = []
  const rebuiltAuxiliaryAgentIds: string[] = []
  let previews: Record<string, string> = {}
  let replaced: { agentId: string | null; text: string[] } | null = null
  const controller = createAgentPaneRefreshController({
    getExpandedTurnIdsByAgent: () => ({}),
    currentAgentPaneEntries: () => [],
    splitAgentResponseMode: () => options.split,
    maxAgentsPerScreen: () => 2,
    loadHistoryPage: async (_sessionId, agentId, cursor) => {
      loads.push(`${agentId}:${cursor ? JSON.stringify(cursor) : "null"}`)
      return {
        entries: [historyEntry(agentId, agentId === "a" ? "hello" : "world\n")],
        nextCursor: null as SessionHistoryCursor | null,
      }
    },
    pruneAuxiliaryAgentPanes: (nextSession) => {
      calls.push(`pruneAuxiliaryAgentPanes:${nextSession.id}`)
    },
    setExpandedTurnIdsByAgent: () => {
      calls.push("setExpandedTurnIdsByAgent")
    },
    setAgentPanePreviews: (nextPreviews) => {
      calls.push("setAgentPanePreviews")
      previews = nextPreviews
    },
    setAgentPaneEntries: () => {
      calls.push("setAgentPaneEntries")
    },
    setNextHistoryCursor: (cursor) => {
      calls.push(`setNextHistoryCursor:${cursor ? "cursor" : "null"}`)
    },
    applyExpandedTurns: (entries) => entries,
    replaceTranscriptEntries: (entries, agentId) => {
      calls.push(`replaceTranscriptEntries:${agentId ?? "none"}`)
      replaced = { agentId, text: entries.map((entry) => entry.text) }
    },
    applyResponseLayout: () => {
      calls.push("applyResponseLayout")
    },
    rebuildAuxiliaryAgentPane: (agentId) => {
      calls.push(`rebuildAuxiliaryAgentPane:${agentId}`)
      rebuiltAuxiliaryAgentIds.push(agentId)
    },
  })

  return {
    calls,
    loads,
    controller,
    get previews() {
      return previews
    },
    get replaced() {
      return replaced
    },
    rebuiltAuxiliaryAgentIds,
  }
}

function session(focusedAgentId: string): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: focusedAgentId,
    max_agents: 2,
    agents: [agent("a"), agent("b")],
    config_state: { values: {} } as RuntimeSession["config_state"],
  }
}

function agent(id: string): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: null,
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
}

function historyEntry(agentId: string, text: string): SessionHistoryPageEntry {
  const kind = agentId === "a" ? "user_prompt" : "provider_output"
  return {
    entry_index: agentId === "a" ? 0 : 1,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: {
      agent_id: agentId,
      kind,
      text,
    },
  }
}
