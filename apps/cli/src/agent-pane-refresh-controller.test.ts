import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeSession,
  TranscriptEntry,
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

test("agent pane refresh controller owns session-change refresh decisions", () => {
  const harness = createHarness({
    split: false,
    currentFocusedAgentId: "a",
  })

  assert.equal(harness.controller.shouldRefreshForSessionChange(session("b")), true)

  const splitHarness = createHarness({
    split: true,
    currentFocusedAgentId: "a",
  })

  assert.equal(splitHarness.controller.shouldRefreshForSessionChange(session("b")), false)
})

test("agent pane refresh backfills history while agent only has queued prompts", async () => {
  const harness = createHarness({
    split: false,
    currentEntries: {
      a: [
        { id: 1, role: "user", turnId: 1, text: "older prompt" },
        { id: 2, role: "assistant", turnId: 1, text: "older answer" },
        { id: 3, role: "user", turnId: 2, text: "latest prompt" },
        { id: 4, role: "assistant", turnId: 2, text: "latest answer" },
      ],
    },
    historyPages: {
      "a:null": {
        entries: [
          { id: 3, role: "user", turnId: 2, text: "latest prompt" },
          { id: 4, role: "assistant", turnId: 2, text: "latest answer" },
        ],
        nextCursor: { before: "latest" },
      },
      "a:{\"before\":\"latest\"}": {
        entries: [
          { id: 1, role: "user", turnId: 1, text: "older prompt" },
          { id: 2, role: "assistant", turnId: 1, text: "older answer" },
        ],
        nextCursor: null,
      },
      "b:null": {
        entries: [historyEntry("b", "world\n")],
        nextCursor: null,
      },
    },
  })

  await harness.controller.refresh(session("a", {
    agent_activity: {
      a: {
        status: "working",
        prompt_status: "queued",
        busy: true,
        active_prompt_count: 0,
        queued_prompt_count: 1,
        unread_idle_output: false,
      },
      b: {
        status: "idle",
        prompt_status: "none",
        busy: false,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
    },
  }))

  assert.deepEqual(harness.loads, ["a:null", "a:{\"before\":\"latest\"}", "b:null"])
  assert.deepEqual(harness.replaced, {
    agentId: "a",
    text: ["older prompt", "older answer", "latest prompt", "latest answer"],
  })
})

test("agent history refresh recovers a completed response without loading unrelated agents", async () => {
  const harness = createHarness({
    split: false,
    currentEntries: {
      a: [{ id: 1, role: "user", turnId: 1, text: "queued remotely" }],
      b: [{ id: 1, role: "assistant", turnId: 1, text: "keep me" }],
    },
    historyPages: {
      "a:null": {
        entries: [
          { id: 1, role: "user", turnId: 1, text: "queued remotely" },
          { id: 2, role: "assistant", turnId: 1, text: "durable response" },
        ],
        nextCursor: null,
      },
    },
  })

  await harness.controller.refreshAgentHistories(session("a"), ["a"])

  assert.deepEqual(harness.loads, ["a:null"])
  assert.deepEqual(harness.replaced, {
    agentId: "a",
    text: ["queued remotely", "durable response"],
  })
  assert.deepEqual(harness.paneEntries.b?.map((entry) => entry.text), ["keep me"])
})

function createHarness(options: {
  split: boolean
  currentFocusedAgentId?: string | null
  currentEntries?: Record<string, TranscriptEntry[]>
  historyPages?: Record<string, { entries: TranscriptEntry[]; nextCursor: unknown }>
}): {
  calls: string[]
  loads: string[]
  controller: ReturnType<typeof createAgentPaneRefreshController>
  previews: Record<string, string>
  paneEntries: Record<string, TranscriptEntry[]>
  replaced: { agentId: string | null; text: string[] } | null
  rebuiltAuxiliaryAgentIds: string[]
} {
  const calls: string[] = []
  const loads: string[] = []
  const rebuiltAuxiliaryAgentIds: string[] = []
  let previews: Record<string, string> = {}
  let paneEntries: Record<string, TranscriptEntry[]> = {}
  let replaced: { agentId: string | null; text: string[] } | null = null
  const controller = createAgentPaneRefreshController({
    getCurrentAgents: () => [agent("a"), agent("b")],
    getFocusedAgentId: () => options.currentFocusedAgentId ?? "a",
    getCollapsedTurnIdsByAgent: () => ({}),
    currentAgentPaneEntries: (agentId) => options.currentEntries?.[agentId] ?? [],
    splitAgentResponseMode: () => options.split,
    maxAgentsPerScreen: () => 2,
    loadHistoryPage: async (_sessionId, agentId, cursor) => {
      loads.push(`${agentId}:${cursor ? JSON.stringify(cursor) : "null"}`)
      const page = options.historyPages?.[`${agentId}:${cursor ? JSON.stringify(cursor) : "null"}`]
      if (page) {
        return {
          entries: page.entries,
          nextCursor: page.nextCursor as never,
        }
      }
      return {
        entries: [historyEntry(agentId, agentId === "a" ? "hello" : "world\n")],
        nextCursor: null,
      }
    },
    pruneAuxiliaryAgentPanes: (nextSession) => {
      calls.push(`pruneAuxiliaryAgentPanes:${nextSession.id}`)
    },
    setCollapsedTurnIdsByAgent: () => {
      calls.push("setCollapsedTurnIdsByAgent")
    },
    setAgentPanePreviews: (nextPreviews) => {
      calls.push("setAgentPanePreviews")
      previews = nextPreviews
    },
    setAgentPaneEntries: (nextEntries) => {
      calls.push("setAgentPaneEntries")
      paneEntries = nextEntries
    },
    setNextHistoryCursor: (cursor) => {
      calls.push(`setNextHistoryCursor:${cursor ? "cursor" : "null"}`)
    },
    applyCollapsedTurns: (entries) => entries,
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
    isCurrentSession: () => true,
  })

  return {
    calls,
    loads,
    controller,
    get previews() {
      return previews
    },
    get paneEntries() {
      return paneEntries
    },
    get replaced() {
      return replaced
    },
    rebuiltAuxiliaryAgentIds,
  }
}

function session(focusedAgentId: string, overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
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
    ...overrides,
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

function historyEntry(agentId: string, text: string): TranscriptEntry {
  const role = agentId === "a" ? "user" : "assistant"
  return {
    id: agentId === "a" ? 1 : 2,
    role,
    text,
  }
}
