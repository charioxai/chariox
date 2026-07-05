import assert from "node:assert/strict"
import test from "node:test"

import {
  agentPromptStateHasWork,
  sessionHasAgent,
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
