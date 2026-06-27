import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionAgentIsBusy,
  sessionHasActivePrompt,
  sessionPromptForAgent,
  sessionPromptWorkSummary,
} from "./shell-agent-activity.js"
import { makeSession } from "./shell-executor.test-support.js"

test("sessionAgentIsBusy uses projected idle over stale legacy prompt state", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
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
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
})

test("sessionAgentIsBusy treats missing projected agent activity as idle", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
})

test("sessionPromptWorkSummary counts projected active turns and prompt state queues", () => {
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
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-2")
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
