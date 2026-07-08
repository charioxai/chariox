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
import { malformedRuntimeValue } from "./session-runtime-projection.test-support.js"

test("sessionPromptWorkSummary counts projected active turns and prompt state queues", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "Queued",
        }],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
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
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-2",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 1,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary prefers projected prompt counts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [{
          id: "stale-queued",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "stale queued",
          status: "Queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_prompt_count: 1,
        queued_prompt_count: 2,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 2,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary ignores settled active turn statuses", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-1" }),
      makeAgent({ id: "agent-2" }),
      makeAgent({ id: "agent-3" }),
    ],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: malformedRuntimeValue("completed"),
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: malformedRuntimeValue(" Completed "),
          phase: malformedRuntimeValue("settled"),
        },
      },
      "agent-2": {
        status: "idle",
        prompt_status: malformedRuntimeValue("cancelled"),
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-2",
          prompt_origin: "external",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
      "agent-3": {
        status: "idle",
        prompt_status: "settling",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-3",
          provider_run_id: "run-3",
          prompt_origin: "external",
          status: malformedRuntimeValue(" settling "),
          phase: "settling",
        },
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary counts prompt state active prompt for sparse busy activity", () => {
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
      "agent-2": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "none",
        busy: true,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary treats prompt states as runtime authority", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-3": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-3",
          source_attachment_id: "attach-3",
          target_agent_id: "agent-3",
          prompt: "queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-2", state: "Idle", is_processing: false }),
      makeAgent({ id: "agent-3", state: "Idle", is_processing: false }),
    ],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 1,
    busyAgents: 1,
  })
})

test("sessionPromptWorkByAgent honors prompt states across agents", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "review",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [
      makeAgent({ id: "agent-1" }),
      makeAgent({ id: "agent-2", is_processing: true, state: "Working" }),
    ],
  })

  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
    "agent-2": true,
  })
})

test("sessionPromptWorkByAgent prefers projected activity over stale prompt state", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
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
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })

  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
    "agent-2": true,
  })
})

test("sessionProjectedStreamingAgentId uses projected activity before legacy active prompts", () => {
  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
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
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-2")

  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    agent_activity: {},
    agents: [makeAgent({ id: "agent-1" })],
  })), null)
})

test("sessionProjectedStreamingAgentId resolves exactly one prompt-state active agent", () => {
  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-2")

  assert.equal(sessionProjectedStreamingAgentId(makeSession({
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
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), null)
})

test("sessionProjectedStreamingAgentId ignores legacy active prompt without projections", () => {
  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    active_prompt: {
      id: "prompt-legacy",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "legacy",
      status: "Running",
    },
    agents: [makeAgent({ id: "agent-1" })],
  })), null)
})

test("resolveSessionStreamingAgentId prefers processing, active prompt, then previous streaming agent", () => {
  const agents = [
    makeAgent({ id: "agent-a", is_processing: false, state: "Idle" }),
    makeAgent({ id: "agent-b", is_processing: true, state: "Working" }),
  ]

  assert.equal(resolveSessionStreamingAgentId(agents, "agent-a", true, true, "agent-a"), "agent-b")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], "agent-a", true, false, null), "agent-a")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], null, true, false, "agent-a"), "agent-a")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], null, false, true, "agent-a"), "agent-a")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], null, false, false, "agent-a"), null)
})

test("resolveSessionStreamingAgentId can ignore legacy processing for projected sessions", () => {
  const agents = [
    makeAgent({ id: "agent-a", is_processing: true, state: "Working" }),
    makeAgent({ id: "agent-b", is_processing: false, state: "Idle" }),
  ]

  assert.equal(resolveSessionStreamingAgentId(agents, "agent-b", true, false, null, false), "agent-b")
  assert.equal(resolveSessionStreamingAgentId(agents, null, true, false, "agent-b", false), "agent-b")
  assert.equal(resolveSessionStreamingAgentId(agents, null, false, true, "agent-b", false), null)
  assert.equal(resolveSessionStreamingAgentId(agents, null, false, false, null, false), null)
})

test("sessionRuntimeTransitionState preserves active labels and clears idle labels", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })
  const nextSession = makeSession({
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
      makeAgent({ id: "agent-2", state: "Working", is_processing: true }),
    ],
    prompt_states: {
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
    nextHasTurnWork: true,
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
