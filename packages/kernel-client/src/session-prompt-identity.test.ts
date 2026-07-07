import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionActivePromptForAgent,
  sessionActivePromptIdForAgent,
  sessionHasActivePrompt,
  sessionHasPendingPrompt,
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

test("session active prompt identity excludes queued-only prompt state", () => {
  const session = makeSession({
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
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-1"), false)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-1"), true)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptStateForAgent(session, "agent-1")?.queued_prompts[0]?.id, "queued-1")
})

test("session active prompt identity excludes legacy queued-only prompts", () => {
  const session = makeSession({
    queued_prompts: [{
      id: "queued-legacy",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "queued",
      status: "Queued",
    }],
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-legacy"), false)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-legacy"), true)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptStateForAgent(session, "agent-1")?.queued_prompts[0]?.id, "queued-legacy")
})

test("session pending prompt identity matches queued pending prompt ids", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-materialized",
          pending_prompt_id: "queued-pending",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "Queued",
        }],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-pending"), false)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-pending"), true)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-materialized"), true)
})

test("session active prompt identity matches active prompt pending ids", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "active-materialized",
          pending_prompt_id: "active-pending",
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
          prompt_id: "active-pending",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "active-pending"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "active-materialized"), true)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "active-materialized"), false)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "active-pending"), true)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), "active-pending")
  assert.equal(sessionActivePromptForAgent(session, "agent-1")?.id, "active-materialized")
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "active-materialized")
})

test("session active prompt identity rejects materialized prompt ids that do not match projected active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "active-materialized",
          pending_prompt_id: "other-pending",
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
          prompt_id: "projected-pending",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "projected-pending"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "active-materialized"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "other-pending"), false)
  assert.equal(sessionActivePromptForAgent(session, "agent-1"), null)
})

test("session pending prompt identity keeps queued prompts visible behind projected active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "running-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [{
          id: "queued-materialized",
          pending_prompt_id: "queued-pending",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "Queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "running-1",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-pending"), false)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "running-1"), true)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-pending"), true)
  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-materialized"), true)
})

test("session pending prompt identity respects projected idle as authoritative", () => {
  const session = makeSession({
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
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

  assert.equal(sessionHasPendingPrompt(session, "agent-1", "queued-stale"), false)
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
