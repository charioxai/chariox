import assert from "node:assert/strict"
import test from "node:test"

import {
  agentRuntimeActiveTurnIsBusy,
  agentRuntimeActivityHasTurnWork,
  agentRuntimeActivityIsBusy,
  agentRuntimeActivityResolvedStatus,
  agentRuntimePromptStatusIsActive,
  agentRuntimePromptStatusIsActivePrompt,
  normalizeAgentRuntimeActivityProjectionStatus,
  normalizeAgentRuntimeActivityStatus,
  normalizeAgentRuntimePromptProjectionStatus,
  normalizeAgentRuntimePromptStatus,
  projectAgentRuntimeActivity,
  readAgentRuntimeCompletedTurn,
} from "./agent-activity.js"

test("agent activity busy helper follows kernel projected activity semantics", () => {
  assert.equal(agentRuntimeActivityIsBusy(null), false)
  assert.equal(agentRuntimeActivityIsBusy(undefined), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "working",
    prompt_status: "none",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: " Working ",
    prompt_status: "none",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "settling",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "dispatching",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: " Cancelling ",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "completed",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "cancelled",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "unknown",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1" },
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1", status: "running" },
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1", status: "completed" },
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1", status: "cancelled" },
  }), false)
  assert.equal(agentRuntimeActiveTurnIsBusy({ prompt_id: "prompt-1" }), true)
  assert.equal(agentRuntimeActiveTurnIsBusy({ prompt_id: "prompt-1", status: "completed" }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: true,
  }), true)
})

test("agent activity helpers normalize status vocabulary", () => {
  assert.equal(normalizeAgentRuntimeActivityStatus(" Working "), "working")
  assert.equal(normalizeAgentRuntimeActivityStatus(""), null)
  assert.equal(normalizeAgentRuntimePromptStatus(" Cancelling "), "cancelling")
  assert.equal(normalizeAgentRuntimePromptStatus(""), null)
  assert.equal(normalizeAgentRuntimeActivityProjectionStatus(" focused "), "idle")
  assert.equal(normalizeAgentRuntimePromptProjectionStatus(" completed "), "none")
  assert.equal(normalizeAgentRuntimePromptProjectionStatus(" Dispatching "), "dispatching")
  assert.equal(normalizeAgentRuntimePromptProjectionStatus(" cancelled "), "none")
  assert.equal(agentRuntimePromptStatusIsActive("queued"), true)
  assert.equal(agentRuntimePromptStatusIsActive("dispatching"), true)
  assert.equal(agentRuntimePromptStatusIsActive("completed"), false)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("queued"), false)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("dispatching"), true)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("running"), true)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("cancelled"), false)
})

test("agent activity resolved status follows error, busy, then idle", () => {
  assert.equal(agentRuntimeActivityResolvedStatus({ status: "error", busy: false }), "error")
  assert.equal(agentRuntimeActivityResolvedStatus({ error: true, status: "idle", busy: false }), "error")
  assert.equal(agentRuntimeActivityResolvedStatus({ status: "working", busy: false }), "working")
  assert.equal(agentRuntimeActivityResolvedStatus({ active_prompt_count: 1 }), "working")
  assert.equal(agentRuntimeActivityResolvedStatus({ status: "idle", busy: false }), "idle")
  assert.equal(agentRuntimeActivityResolvedStatus(null), "idle")
})

test("agent activity turn-work helper distinguishes active turns from queued-only work", () => {
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "working",
    prompt_status: "running",
    busy: true,
  }), true)
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "working",
    prompt_status: "queued",
    busy: true,
    active_prompt_count: 0,
    queued_prompt_count: 1,
  }), false)
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "working",
    prompt_status: "none",
    busy: true,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "working",
    prompt_status: "none",
    busy: true,
    active_turn: {
      prompt_id: "prompt-1",
      status: "running",
      phase: "streaming",
    },
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: {
      status: "running",
    },
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: {
      prompt_id: "prompt-1",
      status: "cancelled",
      phase: "settled",
    },
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), false)
  assert.equal(agentRuntimeActivityHasTurnWork({
    status: "idle",
    prompt_status: "none",
    busy: false,
  }), false)
})

test("agent activity projection preserves kernel counts as activity source", () => {
  assert.deepEqual(projectAgentRuntimeActivity({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_prompt_count: 0,
    queued_prompt_count: 2,
  }), {
    status: "idle",
    promptStatus: "none",
    busy: true,
    activeTurn: null,
    activePromptCount: 0,
    activePromptCountExplicit: true,
    queuedPromptCount: 2,
    queuedPromptCountExplicit: true,
    error: false,
    unreadIdleOutput: false,
  })
})

test("agent activity projection unwraps nested activity and normalizes settled statuses", () => {
  const activeTurn = { prompt_id: "prompt-1", status: "completed" }
  assert.deepEqual(projectAgentRuntimeActivity({
    activity: {
      status: "focused",
      prompt_status: "cancelled",
      busy: false,
      active_turn: activeTurn,
    },
  }), {
    status: "idle",
    promptStatus: "none",
    busy: false,
    activeTurn,
    activePromptCount: 0,
    activePromptCountExplicit: false,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: false,
    error: false,
    unreadIdleOutput: false,
  })
})

test("agent activity projection exposes live active turn identity", () => {
  assert.deepEqual(projectAgentRuntimeActivity({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: {
      prompt_id: "prompt-1",
      provider_run_id: "run-1",
      prompt_origin: " external ",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
      status: " Running ",
      phase: "streaming",
      started_at_ms: 123,
    },
  }), {
    status: "idle",
    promptStatus: "none",
    busy: true,
    activeTurn: {
      prompt_id: "prompt-1",
      provider_run_id: "run-1",
      prompt_origin: " external ",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
      status: " Running ",
      phase: "streaming",
      started_at_ms: 123,
    },
    activeTurnPromptId: "prompt-1",
    activeTurnProviderRunId: "run-1",
    activeTurnPromptOrigin: "external",
    activeTurnExternalProvider: "codex",
    activeTurnExternalProviderSessionId: "thread-1",
    activeTurnExternalProviderTurnId: "turn-1",
    activeTurnStatus: "running",
    activeTurnPhase: "streaming",
    activeTurnStartedAtMs: 123,
    activePromptCount: 1,
    activePromptCountExplicit: false,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: false,
    error: false,
    unreadIdleOutput: false,
  })

  assert.equal(projectAgentRuntimeActivity({
    active_turn: {
      prompt_id: "prompt-completed",
      status: "completed",
    },
  }).activeTurnPromptId, undefined)

  assert.equal(projectAgentRuntimeActivity({
    active_turn: {
      prompt_id: "prompt-external",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      status: "running",
    },
  }).activeTurnPromptOrigin, "external")
})

test("agent activity projection exposes completed turn action metadata", () => {
  const activity = {
    last_completed_turn: {
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      provider_run_id: "run-1",
      agent_id: "agent-1",
      completed_at_ms: 500,
      duration_ms: 120,
      changed_paths: ["src/a.ts", 42, "src/b.ts"],
      undo_available: false,
      undo_unavailable_reason: "turn already undone",
    },
  }

  assert.deepEqual(readAgentRuntimeCompletedTurn(activity), {
    turnId: "turn-1",
    promptId: "prompt-1",
    providerRunId: "run-1",
    agentId: "agent-1",
    completedAtMs: 500,
    durationMs: 120,
    changedPaths: ["src/a.ts", "src/b.ts"],
    undoAvailable: false,
    undoUnavailableReason: "turn already undone",
  })
  assert.deepEqual(projectAgentRuntimeActivity(activity).lastCompletedTurn, {
    turnId: "turn-1",
    promptId: "prompt-1",
    providerRunId: "run-1",
    agentId: "agent-1",
    completedAtMs: 500,
    durationMs: 120,
    changedPaths: ["src/a.ts", "src/b.ts"],
    undoAvailable: false,
    undoUnavailableReason: "turn already undone",
  })
  assert.equal(readAgentRuntimeCompletedTurn({
    last_completed_turn: {
      turn_id: "turn-1",
    },
  }), null)
})

test("agent activity projection exposes unread idle output", () => {
  assert.equal(projectAgentRuntimeActivity({
    activity: {
      status: "idle",
      prompt_status: "none",
      busy: false,
      unread_idle_output: true,
    },
  }).unreadIdleOutput, true)
  assert.equal(projectAgentRuntimeActivity({
    activity: {
      unread_idle_output: false,
    },
  }).unreadIdleOutput, false)
})

test("agent activity projection preserves previous error only when kernel omits error state", () => {
  assert.equal(projectAgentRuntimeActivity({}, { previousError: true }).error, true)
  assert.equal(projectAgentRuntimeActivity({ status: "error" }, { previousError: false }).error, true)
  assert.equal(projectAgentRuntimeActivity({ status: "idle" }, { previousError: true }).error, false)
  assert.equal(projectAgentRuntimeActivity({ error: false }, { previousError: true }).error, false)
})
