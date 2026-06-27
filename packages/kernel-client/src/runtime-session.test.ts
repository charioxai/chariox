import assert from "node:assert/strict"
import test from "node:test"

import { sessionWithProjectedAgentActivity } from "./runtime-session.js"
import { makeSession } from "./shell-executor.test-support.js"

test("sessionWithProjectedAgentActivity preserves projected activity revision", () => {
  const session = sessionWithProjectedAgentActivity({
    session: makeSession(),
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

test("sessionWithProjectedAgentActivity leaves legacy session untouched without projected activity", () => {
  const legacy = makeSession()
  const session = sessionWithProjectedAgentActivity({
    session: legacy,
    agent_activity: null,
    agent_activity_revision: 17,
  })

  assert.equal(session, legacy)
  assert.equal(session.agent_activity_revision, undefined)
})
