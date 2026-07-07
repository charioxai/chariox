import assert from "node:assert/strict"
import test from "node:test"

import type { AgentPromptState } from "./kernel-types.js"
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

test("sessionAgentRuntimeState normalizes projected error status", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: malformedRuntimeValue(" Error "),
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Error")

  assert.equal(agentRuntimeStateFromProjection(makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  }), {
    agentActivity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        error: true,
      },
    },
  }), "Error")
})

test("sessionHasActivePrompt follows projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Working")
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-2")
})

test("session prompt helpers ignore settled projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "external",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionPromptForAgent rejects legacy prompts that do not match projected active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionHasActivePrompt does not invent prompt identity from anonymous projected activity", () => {
  const session = makeSession({
    prompt_states: {},
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers use prompt state identity when projected activity is busy without active turn", () => {
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
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-1")
})

test("sessionHasActivePrompt follows projected active turn even when prompt state is absent", () => {
  const session = makeSession({
    prompt_states: {},
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionHasActivePrompt falls back to legacy fields when projection is unavailable", () => {
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
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-1")
})

test("session prompt helpers prefer explicit empty prompt state over stale top-level active prompt", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionPromptStateForAgent normalizes prompt states with omitted queued prompts", () => {
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
      } as AgentPromptState,
    },
  })

  assert.deepEqual(sessionPromptStateForAgent(session, "agent-1"), {
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "running",
      status: "Running",
    },
    queued_prompts: [],
  })
})

test("session prompt helpers treat missing prompt state agents as idle", () => {
  const session = makeSession({
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
    prompt_states: {},
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptStateForAgent(session, "agent-1"), null)
})

test("sessionPromptStateForAgent ignores legacy prompts once activity projection exists", () => {
  const session = makeSession({
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
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionPromptStateForAgent(session, "agent-1"), null)
})

test("sessionPromptStateForAgent scopes legacy top-level prompts by agent", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "running",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "queued",
      status: "Queued",
    }, {
      id: "queued-other",
      source_attachment_id: "attach-2",
      target_agent_id: "agent-2",
      prompt: "other",
      status: "Queued",
    }],
  })

  assert.deepEqual(sessionPromptStateForAgent(session, "agent-1"), {
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "running",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "queued",
      status: "Queued",
    }],
  })
  assert.equal(sessionPromptStateForAgent(session, "agent-2")?.active_prompt, null)
  assert.equal(sessionPromptStateForAgent(session, null), null)
})

test("session prompt helpers ignore prompt states for agents outside the session", () => {
  const session = makeSession({
    prompt_states: {
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

  assert.equal(sessionAgentIsBusy(session, "agent-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "prompt-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "queued-ghost"), false)
  assert.equal(sessionPromptForAgent(session, "agent-ghost"), null)
})

test("session prompt helpers prefer explicit empty prompt state over stale top-level queued prompts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers ignore top-level prompts for other agents", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-other",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "other",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-other",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "other queued",
      status: "Queued",
    }],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-other"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionActivePromptLifecycleRecords uses projected active turns and deterministic order", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-b", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-a", state: "Working", is_processing: true }),
    ],
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "prompt-a-stale",
          source_attachment_id: "attach-a",
          target_agent_id: "agent-a",
          prompt: "stale",
          status: "Running",
          prompt_origin: "arroba",
        },
        queued_prompts: [],
      },
      "agent-b": {
        active_prompt: {
          id: "prompt-b-state",
          source_attachment_id: "attach-b",
          target_agent_id: "agent-b",
          prompt: "running",
          status: "Running",
          prompt_origin: "external",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-a-live",
          status: "running",
          prompt_origin: "external",
          phase: "streaming",
        },
      },
      "agent-b": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "prompt-a-live",
    status: "running",
    promptOrigin: "external",
    target_agent_id: "agent-a",
  }, {
    id: "prompt-b-state",
    source_attachment_id: "attach-b",
    target_agent_id: "agent-b",
    prompt: "running",
    status: "running",
    prompt_origin: "external",
    promptOrigin: "external",
  }])
})

test("sessionActivePromptIdForAgent prefers projected active turn and per-agent prompt state", () => {
  assert.equal(sessionActivePromptIdForAgent(makeSession({
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
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), "projected-prompt")

  assert.equal(sessionActivePromptIdForAgent(makeSession({
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), "state-prompt")

  assert.equal(sessionActivePromptIdForAgent(makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-idle-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), null)
})

test("sessionActivePromptForAgent returns only the active prompt under projected runtime state", () => {
  assert.equal(sessionActivePromptForAgent(makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "queued",
        }],
      },
    },
  }), "agent-1"), null)

  assert.equal(sessionActivePromptForAgent(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "running",
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  }), "agent-1"), null)

  const activePrompt = {
    id: "prompt-active",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "active",
    status: "running",
  }
  assert.equal(sessionActivePromptForAgent(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "running",
    },
    prompt_states: {
      "agent-1": {
        active_prompt: activePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-active",
          status: "running",
          phase: "streaming",
        },
      },
    },
  }), "agent-1")?.id, "prompt-active")

  assert.equal(sessionActivePromptForAgent(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "running",
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          ...activePrompt,
          id: "prompt-other",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-active",
          status: "running",
          phase: "streaming",
        },
      },
    },
  }), "agent-1"), null)
})

test("sessionActivePromptIdForAgent falls back to prompt state for sparse busy activity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, null), "state-prompt")
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), "state-prompt")
})

test("sessionActivePromptIdForAgent ignores legacy active prompt for sparse activity without prompt state", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale top-level prompt",
      status: "running",
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
})

test("sessionActivePromptIdForAgent suppresses prompt state for idle or missing projected activity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-a",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "stale-b",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-2",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, null), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-2"), null)
})
