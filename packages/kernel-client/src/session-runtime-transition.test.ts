import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionActivePromptIdForAgent,
  sessionActivePromptForAgent,
  sessionHasActivePrompt,
  sessionPromptForAgent,
  sessionPromptStateForAgent,
} from "./session-prompt-identity.js"
import {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
import {
  sessionAgentIsBusy,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
  sessionPromptWorkByAgent,
  sessionPromptWorkSummary,
} from "./session-prompt-work.js"
import {
  runtimeProviderRunForAgent,
  sessionActiveInteractionForAgent,
} from "./session-runtime-lookup.js"
import {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  resolveSessionStreamingAgentId,
  sessionFocusedAgentId,
  sessionRuntimeTransitionState,
  sessionShouldConfirmIdleTurnCompletion,
  sessionWorkingStateAfterTurnWork,
  shouldPreserveAgentActivityLabel,
  turnCompletionDelayMs,
} from "./session-runtime-transition.js"
import {
  agentRuntimeStateFromProjection,
  sessionAgentHasUnreadIdleOutput,
  sessionAgentPaneStatusBadge,
  sessionAgentRuntimeActivityProjection,
  sessionAgentRuntimeActivityStatus,
  sessionAgentRuntimeDisplayState,
  sessionAgentRuntimeState,
  sessionFocusedStatusBadge,
  sessionStatusLabel,
  sessionStatusMode,
} from "./session-runtime-status.js"
import {
  sessionAttachedFooterSummary,
  sessionFooterHint,
  sessionVisibleAgentSummary,
} from "./shell-session-footer.js"
import { makeAgent, makeSession } from "./shell-executor.test-support.js"

test("sessionRuntimeTransitionState clears stale streaming when projected activity is idle", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const nextSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  }), {
    nextFocusedAgentId: "agent-1",
    nextHasPromptWork: false,
    nextHasTurnWork: false,
    nextStreamingAgentId: null,
    nextFocusedActivityLabel: null,
    nextAgentActivityLabels: {
      "agent-1": null,
    },
    nextWorking: true,
    activePromptChanged: false,
    cancelledPromptSettled: false,
    settledAgentIds: [],
    shouldClearWorkingAfterPromptSettlement: false,
    shouldClearCancelledPromptRuntimeResidue: false,
    shouldConfirmTurnCompletionAfterCancelledPromptSettlement: false,
    nextStreamingAgentIdAfterCancelledPromptSettlement: null,
    shouldConfirmIdleTurnCompletion: true,
    previousAgentSignature: "agent-1",
    nextAgentSignature: "agent-1",
  })
})

test("sessionRuntimeTransitionState does not preserve active runtime state for queued-only activity", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const nextSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "queued",
        busy: true,
        active_prompt_count: 0,
        queued_prompt_count: 1,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  }), {
    nextFocusedAgentId: "agent-1",
    nextHasPromptWork: true,
    nextHasTurnWork: false,
    nextStreamingAgentId: null,
    nextFocusedActivityLabel: null,
    nextAgentActivityLabels: {
      "agent-1": null,
    },
    nextWorking: false,
    activePromptChanged: false,
    cancelledPromptSettled: false,
    settledAgentIds: [],
    shouldClearWorkingAfterPromptSettlement: false,
    shouldClearCancelledPromptRuntimeResidue: false,
    shouldConfirmTurnCompletionAfterCancelledPromptSettlement: false,
    nextStreamingAgentIdAfterCancelledPromptSettlement: null,
    shouldConfirmIdleTurnCompletion: true,
    previousAgentSignature: "agent-1",
    nextAgentSignature: "agent-1",
  })
})

test("sessionRuntimeTransitionState resolves streaming from prompt state before stale processing", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })
  const nextSession = makeSession({
    agents: [
      makeAgent({ id: "agent-1", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-2", state: "Idle", is_processing: false }),
    ],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "run",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
  })

  const transition = sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: { "agent-1": "thinking", "agent-2": "writing" },
  })

  assert.equal(transition.nextStreamingAgentId, "agent-2")
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-1": null,
    "agent-2": "writing",
  })
})

test("sessionRuntimeTransitionState treats empty prompt states as authoritative idle", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const nextSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    prompt_states: {},
  })

  const transition = sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  })

  assert.equal(transition.nextHasPromptWork, false)
  assert.equal(transition.nextHasTurnWork, false)
  assert.equal(transition.nextStreamingAgentId, null)
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-1": null,
  })
})

test("sessionWorkingStateAfterTurnWork keeps working latched until turn completion is confirmed", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const activeSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "run",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  })
  const queuedOnlySession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "queued",
        }],
      },
    },
  })

  assert.equal(sessionWorkingStateAfterTurnWork({
    currentWorking: true,
    nextSession: idleSession,
  }), true)
  assert.equal(sessionWorkingStateAfterTurnWork({
    currentWorking: true,
    nextSession: activeSession,
  }), true)
  assert.equal(sessionWorkingStateAfterTurnWork({
    currentWorking: false,
    nextSession: activeSession,
  }), true)
  assert.equal(sessionWorkingStateAfterTurnWork({
    currentWorking: false,
    nextSession: queuedOnlySession,
  }), false)
  assert.equal(sessionWorkingStateAfterTurnWork({
    currentWorking: false,
    nextSession: idleSession,
  }), false)
})

test("agent busy latches set, clear, and preserve unchanged records", () => {
  const empty: Record<string, boolean> = {}
  assert.equal(readAgentBusyLatch(empty, null), false)
  assert.equal(nextAgentBusyLatches(empty, null, true), empty)

  const busy = nextAgentBusyLatches(empty, "agent-1", true)
  assert.deepEqual(busy, { "agent-1": true })
  assert.equal(readAgentBusyLatch(busy, "agent-1"), true)
  assert.equal(nextAgentBusyLatches(busy, "agent-1", true), busy)

  const cleared = nextAgentBusyLatches(busy, "agent-1", false)
  assert.deepEqual(cleared, {})
})

test("agent activity labels preserve current labels only while activity is still authoritative", () => {
  const current = { "agent-1": "writing" }
  assert.deepEqual(nextAgentActivityLabels(current, "agent-1", "reading", false), { "agent-1": "reading" })
  assert.deepEqual(nextAgentActivityLabels(current, "agent-1", null, true), { "agent-1": "writing" })
  assert.deepEqual(nextAgentActivityLabels(current, "agent-1", null, false), { "agent-1": null })
  assert.equal(nextAgentActivityLabels(current, null, "reading", false), current)
})

test("agent activity labels are preserved for streaming and projected prompt work", () => {
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1" })] }),
    streamingAgentId: "agent-1",
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({
      agents: [makeAgent({ id: "agent-1" })],
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
    }),
    streamingAgentId: null,
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1", state: "Working" })] }),
    streamingAgentId: null,
  }), false)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1" })] }),
    streamingAgentId: null,
  }), false)
})

test("projected idle activity suppresses stale legacy busy state", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session,
    streamingAgentId: null,
  }), false)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: false,
    submittingAgentId: null,
    session,
    streamingAgentId: null,
    agentActivityLabels: {},
    agentBusyLatches: {},
  }), [{ id: "agent-1", busy: false }])
})

test("focused activity and busy state derive from labels, latches, and projected prompt work", () => {
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: "agent-1",
    activeToolLabel: "reading",
    agentActivityLabel: "thinking",
  }), "reading")
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: "agent-1",
    activeToolLabel: null,
    agentActivityLabel: "thinking",
  }), "thinking")
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: null,
    activeToolLabel: "reading",
    agentActivityLabel: "thinking",
  }), null)

  const idleSession = makeSession({ agents: [makeAgent({ id: "agent-1" })] })
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session: idleSession,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: { "agent-1": true },
  }), true)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session: makeSession({ agents: [makeAgent({ id: "agent-1", is_processing: true })] }),
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session: idleSession,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
})

test("active tool labels prefer visible transcript tools and ignore completed pane tools", () => {
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "agent-1",
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: ["reading", "patching"],
    agentPaneToolUpdates: null,
  }), "patching")
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "agent-2",
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: ["reading"],
    agentPaneToolUpdates: [
      { tool: "read", status: "completed" },
      { tool: "bash", status: "running" },
      { tool: "edit", status: "error" },
      { tool: "grep", status: "cancelled" },
    ],
  }), "bashing")
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: null,
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: ["reading"],
    agentPaneToolUpdates: null,
  }), null)
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "agent-2",
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: [],
    agentPaneToolUpdates: [{ tool: "custom_tool", status: "running" }],
    toolActivityLabel: (tool?: string | null) => tool ? `custom ${tool}` : null,
  }), "custom custom_tool")
})

test("all agent busy state is derived per agent", () => {
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: true,
    submittingAgentId: "agent-1",
    session: makeSession({
      agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2", state: "Working" })],
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
        },
        "agent-2": {
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
    { id: "agent-1", busy: true },
    { id: "agent-2", busy: true },
  ])
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: false,
    submittingAgentId: null,
    session: makeSession({ agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })] }),
    streamingAgentId: "agent-2",
    agentActivityLabels: { "agent-1": "thinking" },
    agentBusyLatches: {},
  }), [
    { id: "agent-1", busy: true },
    { id: "agent-2", busy: true },
  ])
})

test("sessionShouldConfirmIdleTurnCompletion treats idle snapshots as stale-turn completion", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Focused" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionHasPromptWork(idleSession), false)
  assert.equal(sessionHasProcessingAgent(idleSession), false)
  assert.equal(sessionShouldConfirmIdleTurnCompletion({
    nextSession: idleSession,
    currentWorking: true,
    currentSubmitting: false,
    currentBusyLatches: {},
    currentStreamingAgentId: "agent-1",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
  }), true)
})

test("sessionShouldConfirmIdleTurnCompletion does not override active prompt or projected processing snapshots", () => {
  const activePromptSession = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1", is_processing: false, state: "Focused" })],
  })
  const processingSession = makeSession({
    agents: [makeAgent({ id: "agent-1", is_processing: true, state: "Working" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  for (const nextSession of [activePromptSession, processingSession]) {
    assert.equal(sessionShouldConfirmIdleTurnCompletion({
      nextSession,
      currentWorking: true,
      currentSubmitting: true,
      currentBusyLatches: { "agent-1": true },
      currentStreamingAgentId: "agent-1",
      currentProviderActivityLabel: "thinking",
      currentActiveStatusLabel: "thinking",
    }), false)
  }
})

test("turnCompletionDelayMs waits for prompt work and terminal record flushes", () => {
  const activeSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
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
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(turnCompletionDelayMs({
    session: activeSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), null)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 1,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), null)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: true,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), null)
})

test("turnCompletionDelayMs returns the remaining quiet window after last turn activity", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), 1_400)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 0,
    now: 1_500,
    quietWindowMs: 1_500,
  }), 0)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 2_000,
    now: 1_000,
    quietWindowMs: 1_500,
  }), 1_500)
})

test("sessionPromptWorkSummary ignores prompt states for agents outside the session", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost running",
          status: "Running",
        },
        queued_prompts: [{
          id: "queued-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    ],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})
