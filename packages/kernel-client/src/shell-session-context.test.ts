import assert from "node:assert/strict"
import test from "node:test"

import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"
import { sessionContextAgentId } from "./shell-session-context.js"

test("sessionContextAgentId keeps only session-scoped focused agents", () => {
  assert.equal(sessionContextAgentId(makeSession({
    focused_agent_id: "agent-2",
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-2")
  assert.equal(sessionContextAgentId(makeSession({
    focused_agent_id: "stale-agent",
    agents: [makeAgent({ id: "agent-1" })],
  })), "agent-1")
  assert.equal(sessionContextAgentId(makeSession({
    focused_agent_id: "stale-agent",
    agents: [],
  })), undefined)
})
