import assert from "node:assert/strict"
import test from "node:test"

import { agentTriggersForAgent } from "./agent-trigger-state.js"
import type { AgentTrigger } from "./kernel-types-session.js"

function trigger(overrides: Partial<AgentTrigger> & Pick<AgentTrigger, "id" | "agent_id">): AgentTrigger {
  return {
    source: { kind: "schedule", trigger: { kind: "interval", every_seconds: 600 }, repeat: "every", overlap_policy: "skip", next_run_at_ms: 10_000 },
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
        source: { kind: "schedule", trigger: { kind: "interval", every_seconds: 30 }, repeat: "once", overlap_policy: "skip", next_run_at_ms: 5_000 },
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

test("MP-08/MP-10/MP-11: HTTP trigger strips have a loopback URL and no countdown", () => {
  const [item] = agentTriggersForAgent({ agent_triggers: [trigger({ id: "http", agent_id: "agent-1",
    source: { kind: "http", ingress_id: "local-ingress", port: 32123, route: "/hooks", methods: ["GET", "POST"], parser: { kind: "json" }, mode: "async" } })] }, "agent-1")
  assert.equal(item?.label, "HTTP GET and POST /hooks")
  assert.equal(item?.localUrl, "http://127.0.0.1:32123/local-ingress/")
  assert.equal(item?.nextRunAtMs, null)
})

test("MP-08/MP-10: a notification trigger whose source kernel is down reads offline", () => {
  const notification = (id: string, source_available: boolean, enabled = true) => trigger({ id, agent_id: "agent-1", enabled,
    source: { kind: "notification", source: { workflow: { source_id: "wf" } }, subscription_id: "sub", display_name: "srcwf", source_available } })
  const items = agentTriggersForAgent({ agent_triggers: [notification("up", true), notification("down", false), trigger({ id: "timer", agent_id: "agent-1" })] }, "agent-1")
  assert.deepEqual(items.map((item) => [item.id, item.offline]), [["timer", false], ["down", true], ["up", false]])
})

test("a cron trigger shows its expression and timezone, and a schedule at its max runs shows done", () => {
  const items = agentTriggersForAgent({
    agent_triggers: [
      trigger({ id: "t-cron", agent_id: "agent-1", source: { kind: "schedule", trigger: { kind: "cron", expression: "0 30 9 * * 1-5", timezone: "Europe/Berlin" }, repeat: "every", overlap_policy: "queue", max_runs: 5, next_run_at_ms: 20_000 }, fired: 2 }),
      trigger({ id: "t-done", agent_id: "agent-1", source: { kind: "schedule", trigger: { kind: "interval", every_seconds: 60 }, repeat: "every", overlap_policy: "skip", max_runs: 2, next_run_at_ms: 9_000 }, fired: 2 }),
    ],
  }, "agent-1")
  assert.deepEqual(items.map((item) => [item.id, item.label, item.nextRunAtMs, item.done]), [
    ["t-cron", "0 30 9 * * 1-5 · Europe/Berlin", 20_000, false],
    ["t-done", "every 1m", null, true],
  ])
})
