import assert from "node:assert/strict"
import test from "node:test"

import { withAgentTaskUpdate } from "./agent-task-state.js"
import { formatAgentWakeLines } from "./agent-wake-status.js"
import { formatAgentInspectSummary, formatAgentListSummary } from "./shell-agent-format.js"
import type { AgentTaskExecution, AgentWake, RuntimeSession } from "./kernel-types.js"
import {
  sessionAgentLedgerStatus,
  sessionAgentPaneStatusBadgeForSession,
  sessionAgentTaskStatus,
  sessionFocusedStatusBadge,
} from "./session-runtime-status.js"

const DEADLINE = Date.UTC(2026, 9, 19, 12, 30, 15)

function task(overrides: Partial<AgentTaskExecution> = {}): AgentTaskExecution {
  return {
    task_id: "t1", room_id: "room", owner_user_id: "owner", agent_id: "a1", prompt_id: "t1",
    provider_run_id: "run", revision: 1, blocked_revision: 0, state: "working", reason: "",
    obligations: [], wait: null, last_progress_at_ms: 1, progress_sequence: 0, no_progress_wakes: 0,
    correction_used: false, pending_prompt_id: null, ...overrides,
  }
}

function wake(overrides: Partial<AgentWake> = {}): AgentWake {
  return {
    id: "w1", task_id: "t1", room_id: "room", agent_id: "a1", registration_id: "r1", kind: "timer",
    label: "tick", state: "scheduled", created_at_ms: 1, verified_at_ms: 2, next_due_ms: DEADLINE - 1000,
    interval_ms: null, command: [], match_text: null, matched_at_ms: null, pid: null, exit_code: null,
    summary: "once", detail: null, fire_count: 0, missed_fires: 0, last_fired_at_ms: null,
    last_sequence: null, last_delivery: null, last_delivered_at_ms: null, last_acknowledged_at_ms: null,
    alerted_sequence: null, ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return { id: "room", agents: [{ id: "a1" }], ...overrides } as unknown as RuntimeSession
}

const waiting = task({
  state: "waiting", reason: "build", revision: 2,
  wait: { registration_ids: ["r1"], deadline_ms: DEADLINE, started_at_ms: 1, inbox_cursor: 0, long_wait_notified: false, last_checked_at_ms: 1 },
})

test("a waiting task shows its deadline and next wake; working and finished tasks add nothing", () => {
  const status = sessionAgentLedgerStatus(session({ agent_tasks: [waiting], agent_wakes: [wake()] }), "a1")
  assert.equal(status?.deadline_ms, DEADLINE)
  assert.match(status!.label, /^WAITING ON build UNTIL 2026-10-19T12:30:15.000Z · NEXT WAKE tick AT /)
  assert.equal(sessionAgentLedgerStatus(session({ agent_tasks: [task()] }), "a1"), null)
  assert.equal(sessionAgentLedgerStatus(session({ agent_tasks: [task({ state: "done" }), task({ task_id: "t2", state: "cancelled" })] }), "a1"), null)
  assert.equal(sessionAgentLedgerStatus(session({ agent_tasks: [waiting] }), "other"), null)
})

test("a blocked task wins over a waiting one and counts open obligations", () => {
  const blocked = task({
    task_id: "t2", state: "blocked", reason: "needs owner",
    obligations: [{ id: "o", kind: "wake", resource_id: null, completion_task_id: null, status: "open", dispatch_state: "accepted" }],
  })
  const status = sessionAgentLedgerStatus(session({ agent_tasks: [waiting, blocked] }), "a1")
  assert.deepEqual(status, { label: "BLOCKED: needs owner (1 obligations)", tone: "error" })
})

test("a waiting task without a finite deadline is blocked, never silently dormant", () => {
  const broken = task({ state: "waiting", wait: { ...waiting.wait!, deadline_ms: 0 } })
  const status = sessionAgentLedgerStatus(session({ agent_tasks: [broken] }), "a1")
  assert.deepEqual([status?.label, status?.tone], ["BLOCKED: Invalid wait deadline", "error"])
})

test("the pane badge shows the wait with a static clock and otherwise stays the ordinary badge", () => {
  const waitingSession = session({ agent_tasks: [waiting] })
  assert.equal(sessionAgentTaskStatus(waitingSession, "a1")?.deadline_ms, DEADLINE)
  const agent = { id: "a1", state: "Idle" } as never
  const badge = (value: RuntimeSession) => sessionAgentPaneStatusBadgeForSession({ session: value, agent, activeLabel: null })
  assert.equal(badge(waitingSession).label, "WAITING UNTIL 12:30:15Z")
  assert.equal(badge(session({ agent_tasks: [task({ state: "done" })] })).label, "IDLE")
  assert.equal(badge(session()).label, "IDLE")
})

test("an older wait or block never masks a running turn on the same agent", () => {
  const agent = { id: "a1", state: "Idle" } as never
  const blocked = task({ task_id: "t2", state: "blocked", reason: "needs owner" })
  for (const retained of [waiting, blocked]) {
    const idle = session({ agent_tasks: [retained] })
    const running = session({
      agent_tasks: [retained],
      prompt_states: { a1: { active_prompt: { prompt_id: "p2" } } },
    } as never)
    const badge = (value: RuntimeSession, extra: object = {}) =>
      sessionAgentPaneStatusBadgeForSession({ session: value, agent, activeLabel: null, ...extra })
    assert.equal(badge(running).tone, "working")
    assert.equal(badge(idle, { isStreaming: true }).tone, "working")
    assert.equal(badge(idle, { busyLatch: true }).tone, "working")
    assert.equal(badge(idle, { activeLabel: "Reading" }).tone, "working")
    assert.equal(badge(idle).label, retained.state === "blocked" ? "BLOCKED: needs owner" : "WAITING UNTIL 12:30:15Z")
    const focused = (focusedBusy: boolean) => sessionFocusedStatusBadge({
      attached: true, daemonDisconnected: false, activeStatusLabel: null, focusedBusy, nowMs: DEADLINE - 60000,
      taskStatus: sessionAgentLedgerStatus(idle, "a1"),
    })
    assert.equal(focused(true).tone, "working")
    assert.equal(focused(false).label, retained.state === "blocked" ? "BLOCKED: needs owner" : "WAITING 1:00 UNTIL 12:30:15Z")
  }
})

test("agent list and inspect show a running turn over an older retained wait", () => {
  const agent = { id: "a1", agent_ref: "a1", session_id: "room", provider: "codex", state: "Idle" } as never
  const running = session({
    agent_tasks: [waiting],
    prompt_states: { a1: { active_prompt: { prompt_id: "p2" } } },
  } as never)
  for (const [value, expected] of [[running, /\[Working;/], [session({ agent_tasks: [waiting] }), /\[WAITING ON build/]] as const) {
    assert.match(formatAgentListSummary([agent], [], {}, { session: value }), expected)
  }
  assert.match(formatAgentInspectSummary(agent, [], null, {}, { session: running }).split("\n")[0]!, /\[Working\]/)
  assert.match(formatAgentInspectSummary(agent, [], null, {}, { session: session({ agent_tasks: [waiting] }) }).split("\n")[0]!, /\[WAITING ON build/)
})

test("a task update applies once by revision and only to known agents of its room", () => {
  const base = session({ agent_tasks: [task()] })
  const updated = withAgentTaskUpdate(base, "room", task({ revision: 2, state: "waiting" }))
  assert.deepEqual(updated.agent_tasks?.map((item) => [item.task_id, item.revision]), [["t1", 2]])
  assert.equal(withAgentTaskUpdate(updated, "room", task({ revision: 2, state: "done" })), updated)
  assert.equal(withAgentTaskUpdate(base, "room", task({ agent_id: "stranger", revision: 9 })), base)
  assert.equal(withAgentTaskUpdate(base, "other", task({ room_id: "other", revision: 9 })), base)
  const added = withAgentTaskUpdate(base, "room", task({ task_id: "t2", revision: 1 }))
  assert.deepEqual(added.agent_tasks?.map((item) => item.task_id), ["t1", "t2"])
})

test("wake lines render the driver's summary, proof of life and the last receipt", () => {
  assert.deepEqual(formatAgentWakeLines(session(), null), ["no wakes"])
  const fired = wake({
    state: "fired", fire_count: 1, missed_fires: 2, last_fired_at_ms: DEADLINE, last_delivered_at_ms: DEADLINE,
    last_delivery: "accepted", summary: "every 60s",
  })
  const lines = formatAgentWakeLines(session({ agent_wakes: [fired, wake({ id: "w2", agent_id: "a2", verified_at_ms: null })] }), "a1")
  assert.equal(lines.length, 1)
  assert.equal(lines[0], "timer 'tick' fired every 60s verified | 1x, last fired 12:30:15Z delivered 12:30:15Z acked - (accepted) missed 2")
  const unverified = formatAgentWakeLines(session({ agent_wakes: [wake({ verified_at_ms: null })] }), null)
  assert.match(unverified[0]!, /UNVERIFIED \| 0x, last not fired$/)
})
