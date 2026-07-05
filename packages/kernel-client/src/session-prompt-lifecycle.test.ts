import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("session prompt lifecycle records normalize active prompt status", () => {
  assert.deepEqual(sessionActivePromptLifecycleRecords(makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: " Running ",
      prompt_origin: " External ",
    },
  })), [{
    id: "prompt-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "running",
    prompt_origin: " External ",
    promptOrigin: "external",
  }])
})

test("session prompt lifecycle transition settles normalized cancelling prompts", () => {
  assert.deepEqual(sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: " Cancelling ",
      },
    }),
    makeSession(),
  ), {
    activePromptChanged: true,
    cancelledPromptSettled: true,
    settledPromptRecords: [{
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: "cancelling",
      promptOrigin: null,
    }],
    settledAgentIds: ["agent-1"],
  })
})

test("session prompt lifecycle records ignore activity and prompt states outside session agents", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost-state",
          source_attachment_id: "attachment-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost",
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
      },
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-ghost-live",
          provider_run_id: "run-ghost",
          prompt_origin: "external",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [])
})

test("session prompt lifecycle falls back to prompt state for sparse active activity", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "running",
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

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "state-prompt",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "running",
    promptOrigin: null,
  }])
})

test("session prompt lifecycle records external active turn metadata", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "external:codex:thread-1:turn-1",
          provider_run_id: "run-1",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "turn-1",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "external:codex:thread-1:turn-1",
    status: "running",
    promptOrigin: "external",
    target_agent_id: "agent-1",
    providerRunId: "run-1",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }])
})

test("session prompt lifecycle transition settles external active turns", () => {
  assert.deepEqual(sessionPromptLifecycleTransition(
    makeSession({
      agents: [makeAgent({ id: "agent-1" })],
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "external:codex:thread-1:turn-1",
            external_provider: "codex",
            external_provider_session_id: "thread-1",
            external_provider_turn_id: "turn-1",
            status: "cancelling",
            phase: "streaming",
          },
        },
      },
    }),
    makeSession({
      agents: [makeAgent({ id: "agent-1" })],
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
        },
      },
    }),
  ), {
    activePromptChanged: true,
    cancelledPromptSettled: true,
    settledPromptRecords: [{
      id: "external:codex:thread-1:turn-1",
      status: "cancelling",
      promptOrigin: "external",
      target_agent_id: "agent-1",
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
    }],
    settledAgentIds: ["agent-1"],
  })
})
