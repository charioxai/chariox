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

test("sessionActivePromptIdForAgent ignores settled projected active turn identity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-settled",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptIdForAgent(session, null), null)
})

test("sessionActivePromptLifecycleRecords treats projected idle as authoritative", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "cancelling",
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "cancelling",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [])
})

test("sessionActivePromptLifecycleRecords ignores legacy active prompt without projections", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-legacy",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "legacy",
      status: "Running",
      prompt_origin: " External ",
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [])
})

test("sessionPromptLifecycleTransition detects when a cancelling prompt settles", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-1",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: "cancelling",
          },
          queued_prompts: [],
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition normalizes cancelling prompt status", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-1",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: " Cancelling ",
          },
          queued_prompts: [],
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition treats projected idle activity as prompt settlement", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-1",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: "cancelling",
          },
          queued_prompts: [],
        },
      },
    }),
    makeSession({
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-1",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "stale",
            status: "cancelling",
          },
          queued_prompts: [],
        },
      },
      agent_activity: {},
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition ignores already-settled projected active turns", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-settled",
            status: malformedRuntimeValue("cancelled"),
            phase: malformedRuntimeValue("settled"),
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, false)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, [])
})

test("sessionPromptLifecycleTransition detects normal prompt replacement", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
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
    }),
    makeSession({
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-2",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "next",
            status: "running",
          },
          queued_prompts: [],
        },
      },
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition settles external prompts when they disappear", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-1",
            status: "running",
            prompt_origin: " External ",
            phase: "streaming",
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionActivePromptLifecycleRecords preserves provider identity without inferring prompt ownership", () => {
  assert.deepEqual(sessionActivePromptLifecycleRecords(makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "external:codex:thread-1:user-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "running",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
        },
        queued_prompts: [],
      },
    },
  })), [{
    id: "external:codex:thread-1:user-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "running",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "user-1",
    promptOrigin: null,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "user-1",
  }])
})

test("sessionActivePromptLifecycleRecords preserves projected provider identity without inferring prompt ownership", () => {
  assert.deepEqual(sessionActivePromptLifecycleRecords(makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "external:codex:thread-1:user-1",
          status: "running",
          phase: "streaming",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
        },
      },
    },
  })), [{
    id: "external:codex:thread-1:user-1",
    status: "running",
    promptOrigin: null,
    target_agent_id: "agent-1",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "user-1",
  }])
})

test("sessionActivePromptLifecycleRecords does not merge sparse external prompt ownership", () => {
  assert.deepEqual(sessionActivePromptLifecycleRecords(makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "projected-prompt",
          status: "running",
          phase: "streaming",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "running",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
        },
        queued_prompts: [],
      },
    },
  })), [{
    id: "projected-prompt",
    status: "running",
    promptOrigin: null,
    target_agent_id: "agent-1",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }])
})

test("sessionActivePromptLifecycleRecords merges prompt state when exact external turn identity matches", () => {
  assert.deepEqual(sessionActivePromptLifecycleRecords(makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "projected-prompt",
          status: "running",
          phase: "streaming",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "running",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
        },
        queued_prompts: [],
      },
    },
  })), [{
    id: "projected-prompt",
    status: "running",
    promptOrigin: null,
    target_agent_id: "agent-1",
    source_attachment_id: "attachment-1",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "user-1",
  }])
})

test("sessionPromptLifecycleTransition settles cancelling external prompts", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "cancelling",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-1",
            status: "cancelling",
            prompt_origin: "External",
            phase: "settling",
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})
