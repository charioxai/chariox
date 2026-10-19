import assert from "node:assert/strict"
import test from "node:test"

import { handleAgentSlashCommand, type AgentCommandHandlerDeps } from "./agent-command-handlers.js"
import type { RuntimeSession } from "./cli-types.js"

function wake(id: string, agentId: string, label: string) {
  return {
    id, task_id: "t", room_id: "room", agent_id: agentId, registration_id: "r", kind: "timer", label,
    state: "scheduled", created_at_ms: 1, verified_at_ms: 2, next_due_ms: null as number | null, interval_ms: null,
    summary: "once", detail: null, fire_count: 0, missed_fires: 0, last_fired_at_ms: null, last_sequence: null,
    last_delivery: null, last_delivered_at_ms: null, last_acknowledged_at_ms: null, alerted_sequence: null,
  }
}

function harness() {
  const notices: string[] = []
  const noticeKeys: Array<string | undefined> = []
  const footers: string[] = []
  const session = { id: "room", agents: [], agent_wakes: [wake("w1", "a1", "first"), wake("w2", "a2", "second")] } as unknown as RuntimeSession
  const deps = {
    isAttached: () => true,
    sessionState: () => session,
    appendNotice: (message: string, _emphasis?: string, mergeKey?: string) => { notices.push(message); noticeKeys.push(mergeKey) },
    flashFooter: (message: string) => { footers.push(message) },
    resolveSessionAgent: (ref: string | null | undefined) =>
      ref === "a1" ? { agent: { id: "a1" } } : { error: `unknown agent ${ref}` },
  } as unknown as AgentCommandHandlerDeps
  return { deps, notices, noticeKeys, footers }
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

test("/agent wakes shows when a scheduled wake fires next", async () => {
  const due = Date.UTC(2026, 9, 21, 8, 30, 15)
  const scheduled = harness()
  const session = scheduled.deps.sessionState() as unknown as { agent_wakes: Array<ReturnType<typeof wake>> }
  session.agent_wakes = [
    { ...wake("w1", "a1", "later"), summary: "", next_due_ms: due },
    { ...wake("w2", "a1", "done"), state: "exited", next_due_ms: due },
  ]
  await handleAgentSlashCommand(scheduled.deps, { kind: "agent", args: ["wakes"] } as never)
  const [armed, ended] = scheduled.notices[0]!.split("\n")
  assert.match(armed!, /^timer 'later' scheduled next 2026-10-21 08:30:15Z verified \|/)
  assert.doesNotMatch(ended!, /next /)
})

// MP-08/MP-10: an explicit kernel inspection survives provider history I/O.
test("wakes inspection has a session-scoped kernel projection key", async () => {
  const inspection = harness()
  await handleAgentSlashCommand(inspection.deps, { kind: "agent", args: ["wakes"] } as never)
  assert.match(inspection.noticeKeys[0] ?? "", /^agent-wakes:room:[0-9]+$/)
})
