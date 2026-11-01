import assert from "node:assert/strict"
import test from "node:test"

import { agentTriggersForAgent } from "./agent-trigger-state.js"
import type { AgentTrigger } from "./kernel-types-session.js"

function trigger(overrides: Partial<AgentTrigger> & Pick<AgentTrigger, "id" | "agent_id">): AgentTrigger {
  return {
    source: { kind: "schedule", interval_seconds: 600, repeat: "every", next_run_at_ms: 10_000 },
    busy_policy: "queue",
    prompt: "Check the build.",
    enabled: true,
    created_at_ms: 1,
    fired: 0,
    coalesced: 0,
    ...overrides,
  }
}

test("agent trigger stripe lists one agent's triggers soonest first with policy and state", () => {
  const items = agentTriggersForAgent({
    agent_triggers: [
      trigger({ id: "t-late", agent_id: "agent-1", busy_policy: "steer", fired: 3 }),
      trigger({ id: "t-other", agent_id: "agent-2" }),
      trigger({
        id: "t-soon",
        agent_id: "agent-1",
        source: { kind: "schedule", interval_seconds: 30, repeat: "once", next_run_at_ms: 5_000 },
      }),
      trigger({ id: "t-off", agent_id: "agent-1", enabled: false, last_error: " steer refused " }),
    ],
  }, "agent-1")
  assert.deepEqual(items.map((item) => [item.id, item.label, item.busyPolicy, item.nextRunAtMs]), [
    ["t-soon", "in 30s", "queue", 5_000],
    ["t-late", "every 10m", "steer", 10_000],
    ["t-off", "every 10m", "queue", null],
  ])
  assert.equal(items[1]?.fired, 3)
  assert.equal(items[2]?.enabled, false)
  assert.equal(items[2]?.lastError, "steer refused")
  assert.deepEqual(agentTriggersForAgent(null, "agent-1"), [])
  assert.deepEqual(agentTriggersForAgent({ agent_triggers: [] }, null), [])
})
