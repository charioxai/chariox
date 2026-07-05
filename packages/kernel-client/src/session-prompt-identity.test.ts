import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionActivePromptForAgent,
  sessionActivePromptIdForAgent,
  sessionHasActivePrompt,
  sessionPromptForAgent,
  sessionPromptStateForAgent,
} from "./session-prompt-identity.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("session prompt identity follows projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
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
          prompt_id: "state-prompt",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "state-prompt"), true)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), "state-prompt")
  assert.equal(sessionActivePromptForAgent(session, "agent-1")?.id, "state-prompt")
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "state-prompt")
})

test("session prompt identity falls back to prompt state for sparse busy activity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
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
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), "state-prompt")
  assert.equal(sessionHasActivePrompt(session, "agent-1", "state-prompt"), true)
  assert.equal(sessionPromptStateForAgent(session, "agent-1")?.active_prompt?.id, "state-prompt")
})

test("session prompt identity suppresses stale legacy prompts under idle projection", () => {
  const session = makeSession({
    active_prompt: {
      id: "legacy-prompt",
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
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "legacy-prompt"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptStateForAgent(session, "agent-1"), null)
})

test("session prompt identity suppresses stale legacy prompts under sparse activity projection", () => {
  const session = makeSession({
    active_prompt: {
      id: "legacy-prompt",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    agent_activity: {},
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "legacy-prompt"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptStateForAgent(session, "agent-1"), null)
})

test("session prompt identity ignores prompt state and activity outside session agents", () => {
  const session = makeSession({
    prompt_states: {
      "agent-ghost": {
        active_prompt: {
          id: "state-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "live-ghost",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionPromptStateForAgent(session, "agent-ghost"), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-ghost"), null)
  assert.equal(sessionActivePromptForAgent(session, "agent-ghost"), null)
  assert.equal(sessionPromptForAgent(session, "agent-ghost"), null)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "live-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "state-ghost"), false)
})
