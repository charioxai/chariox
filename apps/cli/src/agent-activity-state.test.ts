import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  shouldPreserveAgentActivityLabel,
} from "@arroba/kernel-client/session-runtime-transition"

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "s1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  } as AgentInstance
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "s1",
    alias: null,
    status: "Active",
    workspace_id: "workspace",
    worktree_id: "worktree",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 1,
    agents: [agent("a1")],
    config_state: { version: 1, values: {} },
    ...overrides,
  } as RuntimeSession
}

test("agent busy latches set, clear, and preserve unchanged records", () => {
  const empty: Record<string, boolean> = {}
  assert.equal(readAgentBusyLatch(empty, null), false)
  assert.equal(nextAgentBusyLatches(empty, null, true), empty)

  const busy = nextAgentBusyLatches(empty, "a1", true)
  assert.deepEqual(busy, { a1: true })
  assert.equal(readAgentBusyLatch(busy, "a1"), true)
  assert.equal(nextAgentBusyLatches(busy, "a1", true), busy)

  const cleared = nextAgentBusyLatches(busy, "a1", false)
  assert.deepEqual(cleared, {})
})

test("agent activity labels preserve current labels only while activity is still authoritative", () => {
  const current = { a1: "writing" }
  assert.deepEqual(nextAgentActivityLabels(current, "a1", "reading", false), { a1: "reading" })
  assert.deepEqual(nextAgentActivityLabels(current, "a1", null, true), { a1: "writing" })
  assert.deepEqual(nextAgentActivityLabels(current, "a1", null, false), { a1: null })
  assert.equal(nextAgentActivityLabels(current, null, "reading", false), current)
})

test("agent activity labels are preserved for streaming and projected prompt work", () => {
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "a1",
    session: session(),
    streamingAgentId: "a1",
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "a1",
    session: session({ agent_activity: { a1: { status: "working", prompt_status: "running", busy: true, unread_idle_output: false } } }),
    streamingAgentId: null,
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "a1",
    session: session({ agents: [agent("a1", { state: "Working" })] }),
    streamingAgentId: null,
  }), false)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "a1",
    session: session(),
    streamingAgentId: null,
  }), false)
})

test("projected idle activity suppresses stale legacy processing state", () => {
  const idleProjectionSession = session({
    agents: [agent("a1", { state: "Working", is_processing: true })],
    agent_activity: {
      a1: {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "a1",
    session: idleProjectionSession,
    streamingAgentId: null,
  }), false)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "a1",
    submitting: false,
    submittingAgentId: null,
    session: idleProjectionSession,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: false,
    submittingAgentId: null,
    session: idleProjectionSession,
    streamingAgentId: null,
    agentActivityLabels: {},
    agentBusyLatches: {},
  }), [{ id: "a1", busy: false }])
})

test("active tool labels prefer visible transcript tools and ignore completed pane tools", () => {
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "a1",
    visibleTranscriptAgentId: "a1",
    activeToolLabels: ["reading", "patching"],
    agentPaneToolUpdates: null,
  }), "patching")
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "a2",
    visibleTranscriptAgentId: "a1",
    activeToolLabels: ["reading"],
    agentPaneToolUpdates: [
      { tool: "read", status: "completed" },
      { tool: "bash", status: "running" },
    ],
  }), "bashing")
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: null,
    visibleTranscriptAgentId: "a1",
    activeToolLabels: ["reading"],
    agentPaneToolUpdates: null,
  }), null)
})

test("focused activity and busy state derive from tool labels, latches, and projected prompt work", () => {
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: "a1",
    activeToolLabel: "reading",
    agentActivityLabel: "thinking",
  }), "reading")
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: "a1",
    activeToolLabel: null,
    agentActivityLabel: "thinking",
  }), "thinking")
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: null,
    activeToolLabel: "reading",
    agentActivityLabel: "thinking",
  }), null)

  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "a1",
    submitting: false,
    submittingAgentId: null,
    session: session(),
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: { a1: true },
  }), true)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "a1",
    submitting: false,
    submittingAgentId: null,
    session: session({ agents: [agent("a1", { is_processing: true })] }),
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "a1",
    submitting: false,
    submittingAgentId: null,
    session: session(),
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
})

test("all agent busy state is derived per agent", () => {
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: true,
    submittingAgentId: "a1",
    session: session({
      agents: [agent("a1"), agent("a2", { state: "Working" })],
      agent_activity: {
        a1: {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
        },
        a2: {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
    }),
    streamingAgentId: null,
    agentActivityLabels: {},
    agentBusyLatches: {},
  }), [
    { id: "a1", busy: true },
    { id: "a2", busy: true },
  ])
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: false,
    submittingAgentId: null,
    session: session({ agents: [agent("a1"), agent("a2")] }),
    streamingAgentId: "a2",
    agentActivityLabels: { a1: "thinking" },
    agentBusyLatches: {},
  }), [
    { id: "a1", busy: true },
    { id: "a2", busy: true },
  ])
})
