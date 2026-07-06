import assert from "node:assert/strict"
import test from "node:test"

import {
  nextAgentBusyLatches,
  providerActivityRuntimeTransition,
  readAgentBusyLatch,
  resolveSessionStreamingAgentId,
  resolveVisibleTranscriptAgentId,
  sessionAuthoritativeIdleTransitionState,
  sessionFocusedAgentId,
  sessionRuntimeTransitionState,
  sessionShouldConfirmIdleTurnCompletion,
  sessionWorkingStateAfterPromptWork,
  turnCompletionDelayMs,
} from "./session-runtime-transition.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("session focused agent keeps only session-scoped focus and falls back without explicit focus", () => {
  assert.equal(sessionFocusedAgentId(makeSession({
    agents: [makeAgent({ id: "agent-a" }), makeAgent({ id: "agent-b" })],
    focused_agent_id: "agent-b",
  })), "agent-b")
  assert.equal(sessionFocusedAgentId(makeSession({
    agents: [makeAgent({ id: "agent-a" })],
    focused_agent_id: "missing",
  })), null)
  assert.equal(sessionFocusedAgentId(makeSession({
    agents: [makeAgent({ id: "agent-a" })],
    focused_agent_id: null,
  })), "agent-a")
  assert.equal(sessionFocusedAgentId({
    agents: [{ id: "agent-a" }, { id: "agent-b" }],
    focused_agent_id: " agent-b ",
  }), "agent-b")
})

test("visible transcript follows focus in individual mode and primary pane in split mode", () => {
  assert.equal(resolveVisibleTranscriptAgentId(false, "agent-a", "agent-b"), "agent-b")
  assert.equal(resolveVisibleTranscriptAgentId(true, "agent-a", "agent-b"), "agent-a")
  assert.equal(resolveVisibleTranscriptAgentId(true, null, "agent-b"), "agent-b")
  assert.equal(resolveVisibleTranscriptAgentId(false, null, null), null)
})

test("session runtime transition preserves active labels and clears idle labels", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })
  const nextSession = makeSession({
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
      makeAgent({ id: "agent-2", state: "Working", is_processing: true }),
    ],
    active_prompt: {
      id: "prompt-2",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "run",
      status: "Running",
    },
    focused_agent_id: "agent-2",
  })

  assert.deepEqual(sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: {
      "agent-1": "thinking",
      "agent-2": "writing",
    },
  }), {
    nextFocusedAgentId: "agent-2",
    nextHasPromptWork: true,
    nextStreamingAgentId: "agent-2",
    nextFocusedActivityLabel: "writing",
    nextAgentActivityLabels: {
      "agent-1": null,
      "agent-2": "writing",
    },
    nextWorking: true,
    activePromptChanged: true,
    cancelledPromptSettled: false,
    settledAgentIds: [],
    shouldClearWorkingAfterPromptSettlement: false,
    shouldClearCancelledPromptRuntimeResidue: false,
    shouldConfirmTurnCompletionAfterCancelledPromptSettlement: false,
    nextStreamingAgentIdAfterCancelledPromptSettlement: "agent-2",
    shouldConfirmIdleTurnCompletion: false,
    previousAgentSignature: "agent-1,agent-2",
    nextAgentSignature: "agent-1,agent-2",
  })
})

test("session runtime transition clears stale projected idle activity", () => {
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

test("session runtime transition reports active prompt settlement", () => {
  const currentSession = makeSession({
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
          prompt_origin: "external",
        },
      },
    },
  })
  const nextSession = makeSession({
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

  const transition = sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  })

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
  assert.equal(transition.shouldClearWorkingAfterPromptSettlement, true)
  assert.equal(transition.shouldClearCancelledPromptRuntimeResidue, false)
  assert.equal(transition.shouldConfirmTurnCompletionAfterCancelledPromptSettlement, false)
  assert.equal(transition.nextStreamingAgentIdAfterCancelledPromptSettlement, null)
  assert.equal(transition.shouldConfirmIdleTurnCompletion, true)
})

test("session runtime transition reports cancelled prompt settlement cleanup", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "cancelling",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          status: "cancelling",
          phase: "settling",
          prompt_origin: "external",
        },
      },
    },
  })
  const nextSession = makeSession({
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

  const transition = sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "cancelling" },
  })

  assert.equal(transition.cancelledPromptSettled, true)
  assert.equal(transition.shouldClearCancelledPromptRuntimeResidue, true)
  assert.equal(transition.shouldConfirmTurnCompletionAfterCancelledPromptSettlement, true)
  assert.equal(transition.nextStreamingAgentIdAfterCancelledPromptSettlement, null)
  assert.equal(transition.shouldConfirmIdleTurnCompletion, true)
})

test("session runtime transition reports idle turn completion from current runtime residue", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })

  const transition = sessionRuntimeTransitionState({
    currentSession: idleSession,
    nextSession: idleSession,
    currentWorking: false,
    currentSubmitting: true,
    currentBusyLatches: { "agent-1": true },
    currentStreamingAgentId: "agent-1",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  })

  assert.equal(transition.nextHasPromptWork, false)
  assert.equal(transition.shouldConfirmIdleTurnCompletion, true)
})

test("session streaming resolution can ignore legacy processing for projected sessions", () => {
  const agents = [
    makeAgent({ id: "agent-a", is_processing: true, state: "Working" }),
    makeAgent({ id: "agent-b", is_processing: false, state: "Idle" }),
  ]

  assert.equal(resolveSessionStreamingAgentId(agents, "agent-b", true, false, null, false), "agent-b")
  assert.equal(resolveSessionStreamingAgentId(agents, null, true, false, "agent-b", false), "agent-b")
  assert.equal(resolveSessionStreamingAgentId(agents, null, false, true, "agent-b", false), null)
})

test("session working and busy latches stay latched until completion is confirmed", () => {
  const empty: Record<string, boolean> = {}
  const busy = nextAgentBusyLatches(empty, "agent-1", true)
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
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

  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: true,
    nextSession: idleSession,
  }), true)
  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: false,
    nextSession: activeSession,
  }), true)
  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: false,
    nextSession: idleSession,
  }), false)
  assert.equal(readAgentBusyLatch(busy, "agent-1"), true)
  assert.deepEqual(nextAgentBusyLatches(busy, "agent-1", false), {})
})

test("session idle turn completion waits only for idle snapshots", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Focused" })],
  })
  const activePromptSession = makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: "running",
    },
    agents: [makeAgent({ id: "agent-1", state: "Focused" })],
  })

  assert.equal(sessionShouldConfirmIdleTurnCompletion({
    nextSession: idleSession,
    currentWorking: true,
    currentSubmitting: false,
    currentBusyLatches: {},
    currentStreamingAgentId: "agent-1",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
  }), true)
  assert.equal(sessionShouldConfirmIdleTurnCompletion({
    nextSession: activePromptSession,
    currentWorking: true,
    currentSubmitting: true,
    currentBusyLatches: { "agent-1": true },
    currentStreamingAgentId: "agent-1",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
  }), false)
})

test("session authoritative idle transition clears only truly idle snapshots", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const activeSession = makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: "running",
    },
    agents: [makeAgent({ id: "agent-1" })],
  })
  const processingSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
  })

  assert.deepEqual(sessionAuthoritativeIdleTransitionState({
    nextSession: idleSession,
    currentStatusLine: "Cancellation requested.",
  }), {
    shouldClearRuntimeResidue: true,
    shouldResetCancellationStatusLine: true,
  })
  assert.deepEqual(sessionAuthoritativeIdleTransitionState({
    nextSession: activeSession,
    currentStatusLine: "Cancellation requested.",
  }), {
    shouldClearRuntimeResidue: false,
    shouldResetCancellationStatusLine: false,
  })
  assert.deepEqual(sessionAuthoritativeIdleTransitionState({
    nextSession: processingSession,
    currentStatusLine: "Cancellation requested.",
  }), {
    shouldClearRuntimeResidue: false,
    shouldResetCancellationStatusLine: false,
  })
})

test("turn completion delay waits for prompt work and record flushes", () => {
  const activeSession = makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: "running",
    },
    agents: [makeAgent({ id: "agent-1" })],
  })
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
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
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), 1_400)
})

test("provider activity runtime transition marks active provider output as working", () => {
  assert.deepEqual(providerActivityRuntimeTransition(true), {
    providerActivityActive: true,
    working: true,
    shouldUpdateSessionChrome: true,
  })
  assert.deepEqual(providerActivityRuntimeTransition(false), {
    providerActivityActive: false,
    working: null,
    shouldUpdateSessionChrome: true,
  })
})
