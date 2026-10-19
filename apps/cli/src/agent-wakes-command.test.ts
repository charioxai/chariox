import assert from "node:assert/strict"
import test from "node:test"

import { handleAgentSlashCommand, type AgentCommandHandlerDeps } from "./agent-command-handlers.js"
import type { RuntimeSession } from "./cli-types.js"

function wake(id: string, agentId: string, label: string) {
  return {
    id, task_id: "t", room_id: "room", agent_id: agentId, registration_id: "r", kind: "timer", label,
    state: "scheduled", created_at_ms: 1, verified_at_ms: 2, next_due_ms: null, interval_ms: null,
    summary: "once", detail: null, fire_count: 0, missed_fires: 0, last_fired_at_ms: null, last_sequence: null,
    last_delivery: null, last_delivered_at_ms: null, last_acknowledged_at_ms: null, alerted_sequence: null,
  }
}

function harness() {
  const notices: string[] = []
  const footers: string[] = []
  const session = { id: "room", agents: [], agent_wakes: [wake("w1", "a1", "first"), wake("w2", "a2", "second")] } as unknown as RuntimeSession
  const deps = {
    isAttached: () => true,
    sessionState: () => session,
    appendNotice: (message: string) => { notices.push(message) },
    flashFooter: (message: string) => { footers.push(message) },
    resolveSessionAgent: (ref: string | null | undefined) =>
      ref === "a1" ? { agent: { id: "a1" } } : { error: `unknown agent ${ref}` },
  } as unknown as AgentCommandHandlerDeps
  return { deps, notices, footers }
}

test("/agent wakes lists every wake, or one agent's wakes", async () => {
  const all = harness()
  await handleAgentSlashCommand(all.deps, { kind: "agent", args: ["wakes"] } as never)
  assert.equal(all.notices.length, 1)
  assert.deepEqual(all.notices[0]!.split("\n").map((line) => line.split(" ")[1]), ["'first'", "'second'"])

  const one = harness()
  await handleAgentSlashCommand(one.deps, { kind: "agent", args: ["wakes", "a1"] } as never)
  assert.deepEqual(one.notices[0]!.split("\n").map((line) => line.split(" ")[1]), ["'first'"])
})

test("/agent wakes refuses an unknown agent without listing anything", async () => {
  const unknown = harness()
  await handleAgentSlashCommand(unknown.deps, { kind: "agent", args: ["wakes", "nobody"] } as never)
  assert.deepEqual(unknown.notices, [])
  assert.deepEqual(unknown.footers, ["unknown agent nobody"])
})
