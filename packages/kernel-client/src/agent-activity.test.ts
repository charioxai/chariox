import assert from "node:assert/strict"
import test from "node:test"

import {
  agentLegacyProcessingStateIsBusy,
  agentRuntimeActiveTurnIsBusy,
  agentRuntimeActivityHasTurnWork,
  agentRuntimeActivityIsBusy,
  agentRuntimeActivityProjectionHasExternalActiveTurn,
  agentRuntimeActivityProjectionResolvedStatus,
  agentRuntimeActivityResolvedStatus,
  agentRuntimeCompletedTurnAlreadyUndone,
  agentRuntimeCompletedTurnCanRestoreUndoAvailability,
  agentRuntimeCompletedTurnIsNewer,
  agentRuntimePromptStatusIsActive,
  agentRuntimePromptStatusIsActivePrompt,
  type AgentRuntimeCompletedTurnActionProjection,
  normalizeAgentRuntimeActivityProjectionStatus,
  normalizeAgentRuntimeActivityStatus,
  normalizeAgentRuntimePromptProjectionStatus,
  normalizeAgentRuntimePromptStatus,
  projectAgentRuntimeActivity,
  readAgentRuntimeCompletedTurn,
  reconcileAgentRuntimeLastCompletedTurn,
} from "./agent-activity.js"

test("legacy processing helper preserves old agent busy fallback semantics", () => {
  assert.equal(agentLegacyProcessingStateIsBusy(null), false)
  assert.equal(agentLegacyProcessingStateIsBusy(undefined), false)
  assert.equal(agentLegacyProcessingStateIsBusy({ is_processing: false, state: "Idle" }), false)
  assert.equal(agentLegacyProcessingStateIsBusy({ is_processing: true, state: "Idle" }), true)
  assert.equal(agentLegacyProcessingStateIsBusy({ is_processing: false, state: "Working" }), true)
  assert.equal(agentLegacyProcessingStateIsBusy({ is_processing: false, state: "Error" }), false)
})

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
  assert.equal(agentRuntimeActivityProjectionResolvedStatus(projectAgentRuntimeActivity({
    error: true,
    status: "idle",
    busy: false,
  })), "error")
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
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
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
    busy: false,
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
  const externalProjection = projectAgentRuntimeActivity({
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
  })
  assert.deepEqual(externalProjection, {
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
  assert.equal(agentRuntimeActivityProjectionHasExternalActiveTurn(externalProjection), true)

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

  assert.equal(projectAgentRuntimeActivity({
    active_turn: {
      prompt_id: "prompt-external",
      external_provider_turn_id: "turn-1",
      status: "running",
    },
  }).activeTurnPromptOrigin, "external")
  assert.equal(agentRuntimeActivityProjectionHasExternalActiveTurn(projectAgentRuntimeActivity({
    active_turn: {
      prompt_id: "prompt-external",
      external_provider_turn_id: "turn-1",
      status: "running",
    },
  })), true)
  assert.equal(agentRuntimeActivityProjectionHasExternalActiveTurn(projectAgentRuntimeActivity({
    active_turn: {
      prompt_id: "prompt-arroba",
      prompt_origin: "arroba",
      status: "running",
    },
  })), false)
  assert.equal(agentRuntimeActivityProjectionHasExternalActiveTurn({
    status: "working",
    promptStatus: "running",
    busy: true,
    activeTurn: null,
    activeTurnExternalProviderSessionId: "thread-1",
    activePromptCount: 1,
    activePromptCountExplicit: true,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: true,
    error: false,
    unreadIdleOutput: false,
  }), true)
  assert.equal(agentRuntimeActivityProjectionHasExternalActiveTurn({
    status: "working",
    promptStatus: "running",
    busy: true,
    activeTurn: null,
    activeTurnPromptOrigin: "arroba",
    activeTurnExternalProviderSessionId: "thread-1",
    activePromptCount: 1,
    activePromptCountExplicit: true,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: true,
    error: false,
    unreadIdleOutput: false,
  }), false)
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

test("completed turn reconciliation preserves local already-undone state for the same turn", () => {
  const alreadyUndone = completedTurnAction({
    undoAvailable: false,
    undoUnavailableReason: "turn already undone",
  })
  const incoming = completedTurnAction({
    undoAvailable: true,
    undoUnavailableReason: null,
  })

  assert.equal(agentRuntimeCompletedTurnAlreadyUndone(alreadyUndone), true)
  assert.equal(agentRuntimeCompletedTurnAlreadyUndone(incoming), false)
  assert.equal(reconcileAgentRuntimeLastCompletedTurn(alreadyUndone, incoming), alreadyUndone)
})

test("completed turn reconciliation keeps incoming snapshots unless current is already undone", () => {
  const current = completedTurnAction({
    completedAtMs: 100,
    undoAvailable: false,
    undoUnavailableReason: "not latest turn",
  })
  const incoming = completedTurnAction({
    completedAtMs: 200,
    undoAvailable: true,
    undoUnavailableReason: null,
  })

  assert.equal(reconcileAgentRuntimeLastCompletedTurn(null, incoming), incoming)
  assert.equal(reconcileAgentRuntimeLastCompletedTurn(current, incoming), incoming)
  assert.equal(reconcileAgentRuntimeLastCompletedTurn(current, null), current)
})

test("completed turn helpers compare freshness and undo restoration eligibility", () => {
  const current = completedTurnAction({
    completedAtMs: 100,
    undoAvailable: false,
    undoUnavailableReason: "pending snapshot",
  })
  const newerSameTurn = completedTurnAction({
    completedAtMs: 200,
    undoAvailable: true,
    undoUnavailableReason: null,
  })
  const newerDifferentTurn = completedTurnAction({
    turnId: "turn-2",
    completedAtMs: 200,
    undoAvailable: true,
    undoUnavailableReason: null,
  })
  const alreadyUndone = completedTurnAction({
    undoAvailable: false,
    undoUnavailableReason: "turn already undone",
  })

  assert.equal(agentRuntimeCompletedTurnIsNewer(null, current), true)
  assert.equal(agentRuntimeCompletedTurnIsNewer(newerSameTurn, current), false)
  assert.equal(agentRuntimeCompletedTurnIsNewer(current, newerSameTurn), true)
  assert.equal(agentRuntimeCompletedTurnCanRestoreUndoAvailability(current, newerSameTurn), true)
  assert.equal(agentRuntimeCompletedTurnCanRestoreUndoAvailability(current, newerDifferentTurn), false)
  assert.equal(agentRuntimeCompletedTurnCanRestoreUndoAvailability(alreadyUndone, newerSameTurn), false)
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

function completedTurnAction(
  overrides: Partial<AgentRuntimeCompletedTurnActionProjection> = {},
): AgentRuntimeCompletedTurnActionProjection {
  return {
    turnId: "turn-1",
    promptId: "prompt-1",
    providerRunId: "run-1",
    agentId: "agent-1",
    completedAtMs: 100,
    durationMs: 50,
    changedPaths: [],
    undoAvailable: false,
    undoUnavailableReason: null,
    ...overrides,
  }
}
