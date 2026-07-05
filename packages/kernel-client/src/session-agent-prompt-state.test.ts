import assert from "node:assert/strict"
import test from "node:test"

import {
  agentPromptStateHasWork,
  sessionHasAgent,
  sessionProjectedPromptActivityEntriesForSessionAgents,
  sessionProjectedPromptActivityForAgent,
  sessionPromptStateEntriesForSessionAgents,
  sessionPromptStateRecordForAgent,
} from "./session-agent-prompt-state.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("session agent prompt state distinguishes absent projection from projected empty state", () => {
  const withoutProjection = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionPromptStateRecordForAgent(withoutProjection, "agent-1"), undefined)

  const withProjection = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  })

  assert.deepEqual(sessionPromptStateRecordForAgent(withProjection, "agent-1"), {
    active_prompt: null,
    queued_prompts: [],
  })
  assert.equal(sessionPromptStateRecordForAgent(withProjection, "agent-2"), null)
})

test("session agent prompt state scopes entries to session agents", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-outside": {
        active_prompt: {
          id: "outside-prompt",
          source_attachment_id: "attach-outside",
          target_agent_id: "agent-outside",
          prompt: "outside",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
  })

  assert.equal(sessionHasAgent(session, "agent-1"), true)
  assert.equal(sessionHasAgent(session, "agent-outside"), false)
  assert.deepEqual(sessionPromptStateEntriesForSessionAgents(session).map(([agentId]) => agentId), ["agent-1"])
})

test("agent prompt state work predicate follows active and queued prompts", () => {
  assert.equal(agentPromptStateHasWork(null), false)
  assert.equal(agentPromptStateHasWork({ active_prompt: null, queued_prompts: [] }), false)
  assert.equal(agentPromptStateHasWork({
    active_prompt: { id: "prompt-1" },
    queued_prompts: [],
  }), true)
  assert.equal(agentPromptStateHasWork({
    active_prompt: null,
    queued_prompts: [{ id: "queued-1" }],
  }), true)
})

test("session projected prompt activity scopes activity to session agents", () => {
  const unprojected = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  assert.equal(sessionProjectedPromptActivityForAgent(unprojected, "agent-1"), null)
  assert.equal(sessionProjectedPromptActivityForAgent(unprojected, "agent-missing"), "not_found")

  const projected = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-outside": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })
  assert.equal(sessionProjectedPromptActivityForAgent(projected, "agent-1"), "idle")
  assert.equal(sessionProjectedPromptActivityForAgent(projected, "agent-outside"), "not_found")

  const active = makeSession({
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
  const projection = sessionProjectedPromptActivityForAgent(active, "agent-1")
  assert.notEqual(projection, null)
  assert.notEqual(projection, "idle")
  assert.notEqual(projection, "not_found")
  if (projection && projection !== "idle" && projection !== "not_found") {
    assert.equal(projection.activeTurnPromptId, "prompt-1")
  }
})

test("session projected prompt activity entries keep active session agent projections only", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-1" }),
      makeAgent({ id: "agent-2" }),
    ],
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
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-outside": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  const entries = sessionProjectedPromptActivityEntriesForSessionAgents(session)
  assert.deepEqual(entries.map(([agentId]) => agentId), ["agent-1"])
  assert.equal(entries[0]?.[1].activeTurnPromptId, "prompt-1")
})

test("session projected prompt activity preserves notable idle projections", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-unread" }),
      makeAgent({ id: "agent-error" }),
      makeAgent({ id: "agent-plain" }),
    ],
    agent_activity: {
      "agent-unread": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
      "agent-error": {
        status: "error",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-plain": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  const unread = sessionProjectedPromptActivityForAgent(session, "agent-unread")
  assert.notEqual(unread, "idle")
  assert.notEqual(unread, "not_found")
  assert.equal(typeof unread === "object" && unread !== null ? unread.unreadIdleOutput : false, true)

  const error = sessionProjectedPromptActivityForAgent(session, "agent-error")
  assert.notEqual(error, "idle")
  assert.notEqual(error, "not_found")
  assert.equal(typeof error === "object" && error !== null ? error.error : false, true)

  assert.equal(sessionProjectedPromptActivityForAgent(session, "agent-plain"), "idle")
})
