import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, PromptQueueItem, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { normalizeRuntimeSession, normalizeRuntimeSessions, normalizeRuntimeSessionWithAgentActivity } from "./cli-types.js"
import { createSessionStateApplyController } from "./session-state-apply-controller.js"
import { getSessionState } from "./session-api.js"
import { deriveWorkflowSelectionState } from "./workflow-selection-sync.js"

test("a lean Room read keeps the loaded workflow, active run details and selection", async () => {
  const workflow: WorkflowDefinition = { id: "workflow-1", alias: "Review", revision: 3, nodes: [{ id: "review", agent_id: "agent-a" }] }
  const run: WorkflowRun = {
    id: "run-1", workflow_id: workflow.id, endpoint_id: "entry", entry_node_id: "review", status: "Running",
    invocation_prompt: "Review the work", active_node_run_id: "node-run-1", node_runs: [], messages: [],
    created_at_ms: 1, started_at_ms: 2, completed_at_ms: null,
  }
  const harness = createHarness({ session: session({ workflows: [workflow], workflow_runs: [run] }) })
  const { workflows, workflow_runs, ...leanSession } = session({ alias: "Updated Room" })
  const client = {
    send: async () => ({ SessionState: {
      session: leanSession,
      processes: [],
      room_workflows: { session_id: leanSession.id, home_kernel_id: "home", revision: "inventory", workflow_count: 1, workflows: [{
        workflow_id: workflow.id, workflow_revision: 3, label: "Review", state: "running", running_count: 1,
        paused_count: 0, queued_count: 0, endpoints: [], runs: [{ run_id: run.id, endpoint_id: "entry", status: "Running", can_pause: true, can_resume: false, can_stop: true }],
      }] },
    } }),
  } as unknown as Parameters<typeof getSessionState>[0]

  harness.controller.apply(await getSessionState(client, leanSession.id))
  const applied = harness.state.session
  const selected = deriveWorkflowSelectionState(applied.workflows ?? [], workflow.id, "review")
  assert.deepEqual({ alias: applied.alias, workflow: selected.workflow, nodeId: selected.nodeId, run: applied.workflow_runs?.[0] }, {
    alias: "Updated Room", workflow, nodeId: "review", run,
  })

  const inventory = applied.room_workflows!
  const cleared = harness.controller.apply({ ...leanSession, room_workflows: { ...inventory, workflow_count: 0, workflows: [] } })
  assert.deepEqual(cleared.workflows, [])
  assert.deepEqual(cleared.workflow_runs, [])

  harness.controller.apply(applied)
  const explicit = harness.controller.apply({ ...applied, workflows: [], workflow_runs: [] })
  assert.deepEqual(explicit.workflows, [])
  assert.deepEqual(explicit.workflow_runs, [])

  harness.controller.apply(applied)
  const other = harness.controller.apply({ ...leanSession, id: "other", room_workflows: { ...inventory, session_id: "other" } })
  assert.deepEqual(other.workflows, [])
  assert.deepEqual(other.workflow_runs, [])
})

test("session state apply controller applies runtime transition and refreshes split focus", () => {
  const current = session({
    focused_agent_id: "agent-a",
    agents: [agent("agent-a"), agent("agent-b")],
  })
  const next = session({
    focused_agent_id: "agent-b",
    agents: [agent("agent-a"), agent("agent-b")],
    config_state: configState("split"),
  })
  const harness = createHarness({
    session: current,
    layout: "individual",
    layoutPreference: "individual",
    working: true,
    streamingAgentId: "agent-a",
    agentActivityLabels: { "agent-a": "thinking" },
  })

  const applied = harness.controller.apply(next)

  assert.equal(applied.id, next.id)
  assert.equal(harness.state.session.id, next.id)
  assert.equal(harness.state.layout, "split")
  assert.equal(harness.state.streamingAgentId, "agent-a")
  assert.equal(harness.state.working, true)
  assert.deepEqual(harness.state.agentActivityLabels, {
    "agent-a": "thinking",
    "agent-b": null,
  })
  assert.deepEqual(harness.calls, [
    "setSession",
    "setAgentActivityLabels",
    "setStreamingAgentId",
    "setResponseLayout",
    "setWorking",
    "confirmAndSchedule",
    "setProviderActivityLabel",
    "setActiveStatusLabel",
    "setSubmitting",
    "promptStop.reset",
    "syncVisibleActivityLabel",
    "updateSessionChrome",
    "refreshSplitPaneFocusRepaint",
  ])
})

test("session state apply controller resets prompt state when active prompt changes", () => {
  const previousPrompt = prompt("prompt-1", "agent-a")
  const nextPrompt = prompt("prompt-2", "agent-a")
  const harness = createHarness({
    session: session({
      prompt_states: {
        "agent-a": {
          active_prompt: previousPrompt,
          queued_prompts: [],
        },
      },
      agents: [agent("agent-a")],
    }),
    submitting: true,
    submittingAgentId: "agent-a",
  })

  harness.controller.apply(session({
    prompt_states: {
      "agent-a": {
        active_prompt: nextPrompt,
        queued_prompts: [],
      },
    },
    agents: [agent("agent-a")],
  }))

  assert.equal(harness.state.submitting, false)
  assert.equal(harness.state.submittingAgentId, null)
  assert.deepEqual(harness.calls.filter((call) => call === "promptStop.reset"), [
    "promptStop.reset",
  ])
  assert.deepEqual(harness.calls.filter((call) => call === "turnCompletion.reset"), [
    "turnCompletion.reset",
  ])
})

test("session state apply controller clears cancelled prompt residue when cancellation settles", () => {
  const cancelledPrompt = prompt("prompt-1", "agent-a", "cancelling")
  const harness = createHarness({
    session: session({
      prompt_states: {
        "agent-a": {
          active_prompt: cancelledPrompt,
          queued_prompts: [],
        },
      },
      agents: [agent("agent-a")],
    }),
    working: true,
    submitting: true,
    submittingAgentId: "agent-a",
    busyLatches: { "agent-a": true },
    streamingAgentId: "agent-a",
    providerActivityLabel: "cancelling",
    activeStatusLabel: "cancelling",
    statusLine: "Cancellation requested.",
    agentActivityLabels: { "agent-a": "cancelling" },
    activeToolLabels: ["tool-1"],
  })

  harness.controller.apply(session({
    prompt_states: {},
    agents: [agent("agent-a")],
  }))

  assert.equal(harness.state.submitting, false)
  assert.equal(harness.state.submittingAgentId, null)
  assert.equal(harness.state.working, false)
  assert.equal(harness.state.streamingAgentId, null)
  assert.equal(harness.state.providerActivityLabel, null)
  assert.equal(harness.state.activeStatusLabel, null)
  assert.equal(harness.state.statusLine, "")
  assert.deepEqual(harness.state.activeToolLabels, [])
  assert.deepEqual(harness.state.clearedBusyAgents, ["agent-a"])
  assert.ok(harness.calls.includes("confirm"))
  assert.ok(harness.calls.includes("cancelPendingTurnCompletion"))
})

test("session state apply controller preserves unrelated status line when cancellation settles", () => {
  const cancelledPrompt = prompt("prompt-1", "agent-a", "cancelling")
  const harness = createHarness({
    session: session({
      active_prompt: cancelledPrompt,
      agents: [agent("agent-a")],
    }),
    working: true,
    submitting: true,
    submittingAgentId: "agent-a",
    busyLatches: { "agent-a": true },
    streamingAgentId: "agent-a",
    providerActivityLabel: "cancelling",
    activeStatusLabel: "cancelling",
    statusLine: "Reconnecting...",
    agentActivityLabels: { "agent-a": "cancelling" },
    activeToolLabels: ["tool-1"],
  })

  harness.controller.apply(session({
    active_prompt: null,
    agents: [agent("agent-a")],
  }))

  assert.equal(harness.state.statusLine, "Reconnecting...")
  assert.deepEqual(harness.calls.filter((call) => call === "setStatusLine"), [])
})

test("session state apply controller clears agent busy when external prompt disappears from active state", () => {
  const externalWorkingPrompt = session({
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          status: "running",
          phase: "streaming",
          prompt_origin: "external",
        },
      },
    },
  })

  const harness = createHarness({
    session: externalWorkingPrompt,
    working: true,
    busyLatches: { "agent-a": true },
  })

  harness.controller.apply(session())

  assert.equal(harness.state.working, false)
  assert.deepEqual(harness.state.clearedBusyAgents, ["agent-a"])
  assert.deepEqual(harness.calls.filter((call) => call === "clearAgentBusy"), [
    "clearAgentBusy",
  ])
})

test("session state apply controller schedules settled turn completion when only queued prompts remain", () => {
  const current = session({
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })
  const queuedOnly = session({
    prompt_states: {
      "agent-a": {
        active_prompt: null,
        queued_prompts: [prompt("queued-1", "agent-a", "Queued")],
      },
    },
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "queued",
        busy: true,
        active_prompt_count: 0,
        queued_prompt_count: 1,
        unread_idle_output: false,
      },
    },
  })
  const harness = createHarness({
    session: current,
    working: true,
    submitting: true,
    submittingAgentId: "agent-a",
    streamingAgentId: "agent-a",
    agentActivityLabels: { "agent-a": "thinking" },
  })

  harness.controller.apply(queuedOnly)

  assert.equal(harness.state.working, false)
  assert.equal(harness.state.submitting, false)
  assert.equal(harness.state.submittingAgentId, null)
  assert.deepEqual(harness.calls.filter((call) => call === "turnCompletion.reset"), [])
  assert.deepEqual(harness.calls.filter((call) => call === "confirmAndSchedule"), [
    "confirmAndSchedule",
  ])
  assert.deepEqual(harness.calls.filter((call) => call === "promptStop.reset"), [
    "promptStop.reset",
  ])
})

function createHarness(options: {
  session: RuntimeSession
  layout?: "individual" | "split"
  layoutPreference?: "individual" | "split" | null
  working?: boolean
  submitting?: boolean
  submittingAgentId?: string | null
  busyLatches?: Record<string, boolean>
  streamingAgentId?: string | null
  providerActivityLabel?: string | null
  activeStatusLabel?: string | null
  statusLine?: string
  agentActivityLabels?: Record<string, string | null>
  activeToolLabels?: string[]
}) {
  const calls: string[] = []
  const state = {
    session: options.session,
    layout: options.layout ?? "individual",
    layoutPreference: options.layoutPreference ?? null,
    working: options.working ?? false,
    submitting: options.submitting ?? false,
    submittingAgentId: options.submittingAgentId ?? null,
    busyLatches: options.busyLatches ?? {},
    streamingAgentId: options.streamingAgentId ?? null,
    providerActivityLabel: options.providerActivityLabel ?? null,
    activeStatusLabel: options.activeStatusLabel ?? null,
    statusLine: options.statusLine ?? "",
    agentActivityLabels: options.agentActivityLabels ?? {},
    activeToolLabels: options.activeToolLabels ?? [],
    clearedBusyAgents: [] as string[],
    turnConfirmed: false,
  }
  const controller = createSessionStateApplyController({
    getSession: () => state.session,
    setSession: (nextSession) => {
      calls.push("setSession")
      state.session = nextSession
    },
    getFocusedAgentId: () => state.session.focused_agent_id ?? state.session.agents[0]?.id ?? null,
    getCurrentResponseLayout: () => state.layout,
    getLayoutPreference: () => state.layoutPreference,
    setResponseLayout: (layout) => {
      calls.push("setResponseLayout")
      state.layout = layout
    },
    getWorking: () => state.working,
    setWorking: (working) => {
      calls.push("setWorking")
      state.working = working
    },
    getSubmitting: () => state.submitting,
    setSubmitting: (submitting) => {
      calls.push("setSubmitting")
      state.submitting = submitting
    },
    clearSubmittingAgentId: () => {
      calls.push("clearSubmittingAgentId")
      state.submittingAgentId = null
    },
    getAgentBusyLatches: () => state.busyLatches,
    getAgentActivityLabels: () => state.agentActivityLabels,
    setAgentActivityLabels: (labels) => {
      calls.push("setAgentActivityLabels")
      state.agentActivityLabels = labels
    },
    clearAgentBusy: (agentId) => {
      calls.push("clearAgentBusy")
      state.clearedBusyAgents.push(agentId)
      state.busyLatches = {
        ...state.busyLatches,
        [agentId]: false,
      }
    },
    getStreamingAgentId: () => state.streamingAgentId,
    setStreamingAgentId: (agentId) => {
      calls.push("setStreamingAgentId")
      state.streamingAgentId = agentId
    },
    getProviderActivityLabel: () => state.providerActivityLabel,
    setProviderActivityLabel: (label) => {
      calls.push("setProviderActivityLabel")
      state.providerActivityLabel = label
    },
    getActiveStatusLabel: () => state.activeStatusLabel,
    setActiveStatusLabel: (label) => {
      calls.push("setActiveStatusLabel")
      state.activeStatusLabel = label
    },
    getStatusLine: () => state.statusLine,
    setStatusLine: (line) => {
      calls.push("setStatusLine")
      state.statusLine = line
    },
    clearActiveToolLabels: () => {
      calls.push("clearActiveToolLabels")
      state.activeToolLabels = []
    },
    turnCompletion: {
      reset: () => {
        calls.push("turnCompletion.reset")
      },
      isConfirmed: () => state.turnConfirmed,
      confirm: () => {
        calls.push("confirm")
        state.turnConfirmed = true
      },
      confirmAndSchedule: () => {
        calls.push("confirmAndSchedule")
      },
    },
    cancelPendingTurnCompletion: () => {
      calls.push("cancelPendingTurnCompletion")
    },
    promptStop: {
      reset: () => {
        calls.push("promptStop.reset")
      },
    },
    syncQueuedPromptEntries: () => {},
    syncVisibleActivityLabel: () => {
      calls.push("syncVisibleActivityLabel")
    },
    updateSessionChrome: () => {
      calls.push("updateSessionChrome")
    },
    refreshSplitPaneFocusRepaint: () => {
      calls.push("refreshSplitPaneFocusRepaint")
    },
  })
  return { calls, state, controller }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "opencode",
      model: "gpt-5.4",
      effort: "medium",
      account_profile: null,
      execution_mode: "build",
      permission_level: "yolo",
    },
    active_provider_run_id: null,
    connected_cli_count: 0,
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    agents: [agent("agent-a")],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: configState(null),
    ...overrides,
  }
}

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.4",
    effort: "medium",
    worktree_id: "/workspace",
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

function prompt(
  id: string,
  targetAgentId: string,
  status = "running",
): PromptQueueItem {
  return {
    id,
    source_attachment_id: "attachment-1",
    target_agent_id: targetAgentId,
    prompt: "test prompt",
    status,
  }
}

function configState(layout: "individual" | "split" | null): RuntimeSession["config_state"] {
  return {
    version: 1,
    values: layout ? { "ui.multiAgentResponseLayout": layout } : {},
    updated_by_attachment_id: null,
  }
}


test("archive snapshots hide panes on initial load, activity refresh and reconnect", () => {
  const incoming = session({ agents: [agent("active"), agent("archived", { archived: true })] })
  for (const value of [normalizeRuntimeSession(incoming), normalizeRuntimeSessions([incoming])[0]!, normalizeRuntimeSessionWithAgentActivity({ session: incoming })]) {
    assert.deepEqual(value.agents.map(agent => agent.id), ["active"])
  }
  assert.equal(incoming.agents.length, 2, "wire snapshot remains available to history readers")
})
