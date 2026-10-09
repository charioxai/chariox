// MP-08 / MP-09 / MP-10 / MP-11 A03: wake visibility shared by TUI and web.
import assert from "node:assert/strict"
import test from "node:test"
import type { AgentTaskExecution, AgentWake, RuntimeSession } from "./kernel-types-session.js"
import { formatAgentWakeLines, sessionAgentNextWake } from "./agent-wake-status.js"
import { sessionAgentTaskStatus } from "./session-runtime-status.js"

function wake(overrides: Partial<AgentWake>): AgentWake {
  return { id: "w", task_id: "t", room_id: "room", agent_id: "agent", registration_id: "completion-w", kind: "timer",
    label: "check-in", state: "scheduled", created_at_ms: 0, verified_at_ms: 1, next_due_ms: Date.UTC(2026, 9, 7, 12, 30),
    interval_ms: null, command: [], match_text: null, matched_at_ms: null, pid: null, exit_code: null, fire_count: 0,
    missed_fires: 0, last_fired_at_ms: null, last_sequence: null, last_delivery: null, last_delivered_at_ms: null,
    last_acknowledged_at_ms: null, alerted_sequence: null, ...overrides }
}
const waiting = { task_id: "t", room_id: "room", owner_user_id: "o", agent_id: "agent", prompt_id: "t", provider_run_id: null,
  revision: 1, blocked_revision: 0, state: "waiting", reason: "timers", obligations: [],
  wait: { registration_ids: ["completion-w"], deadline_ms: Date.UTC(2026, 9, 7, 13), started_at_ms: 0, inbox_cursor: 0, long_wait_notified: false, last_checked_at_ms: 0 },
  last_progress_at_ms: 0, progress_sequence: 0, no_progress_wakes: 0, correction_used: false, pending_prompt_id: null } satisfies AgentTaskExecution

test("MP-08/MP-09/MP-10/MP-11 A03 waiting status names the next armed wake", () => {
  const session = { agent_tasks: [waiting], agent_wakes: [
    wake({ id: "late", label: "deploy", next_due_ms: Date.UTC(2026, 9, 7, 12, 59) }),
    wake({ id: "soon" }),
    wake({ id: "done", state: "fired", next_due_ms: null }),
  ] } as unknown as RuntimeSession
  assert.equal(sessionAgentNextWake(session, "agent")?.id, "soon")
  assert.match(sessionAgentTaskStatus(session, "agent")!.label, /NEXT WAKE check-in AT 2026-10-07T12:30:00\.000Z/)
})

test("MP-08/MP-09/MP-10/MP-11 A03 wake list shows proof of life and receipts", () => {
  const session = { agent_wakes: [
    wake({ verified_at_ms: null, interval_ms: 60000 }),
    wake({ id: "f", state: "fired", next_due_ms: null, fire_count: 1, last_fired_at_ms: Date.UTC(2026, 9, 7, 12, 0, 1),
      last_delivered_at_ms: Date.UTC(2026, 9, 7, 12, 0, 2), last_acknowledged_at_ms: Date.UTC(2026, 9, 7, 12, 0, 9), last_delivery: "handled" }),
  ] } as RuntimeSession
  const [armed, fired] = formatAgentWakeLines(session, "agent")
  assert.match(armed!, /timer 'check-in' scheduled next 12:30:00Z every 60s UNVERIFIED \| 0x, last not fired/)
  assert.match(fired!, /fired 12:00:01Z delivered 12:00:02Z acked 12:00:09Z \(handled\)/)
  assert.deepEqual(formatAgentWakeLines(session, "other"), ["no wakes"])
})
