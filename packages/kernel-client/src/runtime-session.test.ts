import assert from "node:assert/strict"
import test from "node:test"

import { sessionWithProjectedAgentActivity } from "./runtime-session.js"
import { makeSession } from "./shell-executor.test-support.js"

test("sessionWithProjectedAgentActivity preserves projected activity revision", () => {
  const session = sessionWithProjectedAgentActivity({
    session: {
      ...makeSession(),
      queued_prompts: null as never,
      active_interactions: null as never,
      metaagent_tasks: null as never,
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agent_activity_revision: 17,
  })

  assert.deepEqual(session.queued_prompts, [])
  assert.deepEqual(session.active_interactions, [])
  assert.deepEqual(session.metaagent_tasks, [])
  assert.deepEqual(session.agent_activity, {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
      unread_idle_output: false,
    },
  })
  assert.equal(session.agent_activity_revision, 17)
})

test("sessionWithProjectedAgentActivity normalizes legacy session shape without projected activity", () => {
  const legacy = {
    ...makeSession(),
    queued_prompts: null as never,
    active_interactions: null as never,
    metaagent_tasks: null as never,
  }
  const session = sessionWithProjectedAgentActivity({
    session: legacy,
    agent_activity: null,
    agent_activity_revision: 17,
  })

  assert.notEqual(session, legacy)
  assert.deepEqual(session.queued_prompts, [])
  assert.deepEqual(session.active_interactions, [])
  assert.deepEqual(session.metaagent_tasks, [])
  assert.equal(session.agent_activity_revision, undefined)
})
