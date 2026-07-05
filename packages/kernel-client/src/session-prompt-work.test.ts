import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionAgentBusyForProviderRunRecovery,
  sessionAgentIsBusy,
  sessionProjectedStreamingAgentId,
  sessionQueuedPromptCount,
  sessionPromptWorkByAgent,
  sessionPromptWorkSummary,
} from "./session-prompt-work.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("session prompt work summary treats prompt states as runtime authority", () => {
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
    busyAgents: 2,
  })
  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
    "agent-2": true,
    "agent-3": true,
  })
  assert.equal(sessionQueuedPromptCount(session), 1)
  assert.equal(sessionQueuedPromptCount(session, "agent-1"), 0)
  assert.equal(sessionQueuedPromptCount(session, "agent-3"), 1)
})

test("session prompt work prefers projected activity over stale prompt state", () => {
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

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionAgentIsBusy(session, "agent-2"), true)
  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
    "agent-2": true,
  })
  assert.equal(sessionQueuedPromptCount(session, "agent-1"), 0)
  assert.equal(sessionAgentBusyForProviderRunRecovery(session, "agent-1"), false)
  assert.equal(sessionAgentBusyForProviderRunRecovery(session, "agent-2"), true)
})

test("session provider run recovery busy state falls back only without runtime projection", () => {
  const legacyOnly = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
  })
  assert.equal(sessionAgentBusyForProviderRunRecovery(legacyOnly, "agent-1"), null)

  const promptStateProjected = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  })
  assert.equal(sessionAgentBusyForProviderRunRecovery(promptStateProjected, "agent-1"), false)
})

test("session prompt work ignores projected activity outside session agents", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_prompt_count: 1,
        queued_prompt_count: 3,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-ghost",
          provider_run_id: "run-ghost",
          prompt_origin: "external",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
  })
  assert.equal(sessionAgentIsBusy(session, "agent-ghost"), false)
  assert.equal(sessionProjectedStreamingAgentId(session), null)
  assert.equal(sessionQueuedPromptCount(session), 0)
  assert.equal(sessionQueuedPromptCount(session, "agent-ghost"), 0)
})

test("session projected streaming agent ignores prompt states outside session agents", () => {
  const session = makeSession({
    prompt_states: {
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionProjectedStreamingAgentId(session), null)
})

test("session queued prompt count uses explicit projected activity counts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "stale-queued",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "queued",
        }],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "queued",
        busy: true,
        queued_prompt_count: 2,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionQueuedPromptCount(session), 2)
  assert.equal(sessionQueuedPromptCount(session, "agent-1"), 2)
  assert.equal(sessionQueuedPromptCount(session, "agent-2"), 0)
})

test("session projected streaming agent follows projected activity before legacy prompts", () => {
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
    agent_activity: {
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })), null)
})

test("session projected streaming agent ignores queued-only projected activity", () => {
  const session = makeSession({
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
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 1,
    busyAgents: 1,
  })
  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionProjectedStreamingAgentId(session), null)
})
