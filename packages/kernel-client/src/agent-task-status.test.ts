// MP-08 / MP-09 / MP-10 / MP-11 A02, supplementary projection regressions.
import assert from "node:assert/strict"
import test from "node:test"
import type { AgentTaskExecution, RuntimeSession } from "./kernel-types-session.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { sessionAgentTaskStatus, sessionFocusedStatusBadge } from "./session-runtime-status.js"
import { agentTaskStatus } from "@chariox/tool-display/agent-task-status"
function task(state: AgentTaskExecution["state"]): AgentTaskExecution {
  return { task_id: state, room_id: "room", owner_user_id: "owner", agent_id: "agent", prompt_id: state,
    provider_run_id: null, revision: 1, blocked_revision: 0, state, reason: "delegate result",
    obligations: [{ id: "child", kind: "delegate", resource_id: "child", completion_task_id: null, status: "open", dispatch_state: "accepted" }],
    wait: { registration_ids: ["reg"], deadline_ms: 60000, started_at_ms: 0, inbox_cursor: 0, long_wait_notified: false, last_checked_at_ms: 0 },
    last_progress_at_ms: 0, progress_sequence: 0, no_progress_wakes: 0, correction_used: false, pending_prompt_id: null }
}
test("rejected dispatch does not remain outstanding in the footer", () => {
  const done = task("done")
  done.wait = null
  done.obligations[0]!.status = "failed"
  done.obligations[0]!.dispatch_state = "rejected"
  assert.equal(agentTaskStatus({ agent_tasks: [done] }, "agent")!.label, "DONE")
  done.state = "working"
  done.obligations[0]!.dispatch_state = "accepted"
  assert.equal(agentTaskStatus({ agent_tasks: [done] }, "agent")!.label, "WORKING (1 obligations)")
})
test("completed refusal goal has zero outstanding obligations", () => {
  const done = { ...task("done"), wait: null, goal: { id: "goal:done", objective: "report the refusal", paused: false,
    active_elapsed_ms: 1, waiting_elapsed_ms: 0, state_since_ms: 1, check_in_minutes: 10,
    next_check_in_ms: null, check_count: 1, evaluation: null } } as unknown as AgentTaskExecution
  done.obligations[0]!.status = "failed"
  done.obligations[0]!.dispatch_state = "rejected"
  assert.match(sessionAgentTaskStatus({ agent_tasks: [done] } as RuntimeSession, "agent", 1)!.label, /0 obligations/)
})
test("MP-08/MP-10/MP-11 A02 newer done cannot hide older waiting task", () => {
  const session = { agent_tasks: [task("waiting"), { ...task("done"), obligations: [] }] } as RuntimeSession
  assert.match(sessionAgentTaskStatus(session, "agent")!.label, /^WAITING ON delegate result UNTIL .*\(1 obligations\)$/)
})
test("MP-08/MP-10/MP-11 A02 blocked takes priority and retained obligations remain visible", () => {
  const session = { agent_tasks: [task("waiting"), task("blocked")] } as RuntimeSession
  assert.deepEqual(sessionAgentTaskStatus(session, "agent"), { label: "BLOCKED: delegate result (2 obligations)", tone: "error" })
  assert.equal(sessionAgentTaskStatus(session, "foreign"), null)
})

test("MP-08/MP-10/MP-11 A02 task DTO requires protocol452",()=>{ assert.ok(LOCAL_DAEMON_PROTOCOL_VERSION >= 452) })


test("MP-08/MP-09/MP-10/MP-11 A02 malformed wait is safely blocked", () => {
  const session = { agent_tasks: [{ ...task("waiting"), wait: null }] } as RuntimeSession
  assert.match(sessionAgentTaskStatus(session, "agent")!.label, /^BLOCKED: Invalid wait deadline/)
})

// MP-08/MP-10/MP-11 G13: actual single-pane status must show the durable wait.
test("focused footer does not call a durable waiting task idle", () => {
  const session = { agent_tasks: [task("waiting")] } as RuntimeSession
  const options = { attached: true, daemonDisconnected: false, activeStatusLabel: null, nowMs: 0,
    focusedBusy: false, agents: [{ id: "agent", busy: false }],
    taskStatus: sessionAgentTaskStatus(session, "agent") }
  assert.equal(sessionFocusedStatusBadge(options).label, "WAITING 1:00 UNTIL 00:01:00Z")
})

// MP-08/MP-09/MP-10/MP-11 G13: long reasons must not consume the terminal footer.
test("focused waiting deadline stays readable with long reasons and multiple agents", () => {
  const session = { agent_tasks: [{ ...task("waiting"), reason: "source review ".repeat(100) }] } as RuntimeSession
  const options = { attached: true, daemonDisconnected: false, activeStatusLabel: null, nowMs: 0,
    focusedBusy: false, agents: [{ id: "agent", busy: false }, { id: "peer", busy: true }],
    taskStatus: sessionAgentTaskStatus(session, "agent") }
  assert.equal(sessionFocusedStatusBadge(options).label, "WAITING 1:00 UNTIL 00:01:00Z")
})

test("MP-08/MP-09/MP-10/MP-11 G13 deadline countdown advances without settling the task", () => {
  const session = { agent_tasks: [task("waiting")] } as RuntimeSession
  const options = { attached: true, daemonDisconnected: false, activeStatusLabel: null,
    focusedBusy: false, taskStatus: sessionAgentTaskStatus(session, "agent") }
  assert.match(sessionFocusedStatusBadge({ ...options, nowMs: 30000 }).label, /WAITING 0:30 UNTIL /)
  assert.match(sessionFocusedStatusBadge({ ...options, nowMs: 60001 }).label, /WAITING due now UNTIL /)
})
// MP-08 / MP-09 / MP-10 / MP-11 #971 R3: current ordinary work wins over history.
for (const terminal of ["done", "cancelled"] as const) {
  for (const current of ["working", "blocked"] as const) {
    test(`terminal ${terminal} Goal cannot hide current ${current} ordinary task`, () => {
      const history = { ...task(terminal), obligations: [], goal: { id:"goal:history", objective:"old", paused:false, active_elapsed_ms:0, waiting_elapsed_ms:0, state_since_ms:0, check_in_minutes:10, next_check_in_ms:null, check_count:1, evaluation:null } } as unknown as AgentTaskExecution
      const session = { agent_tasks:[history,task(current)] } as RuntimeSession
      assert.deepEqual(sessionAgentTaskStatus(session,"agent",1), current === "blocked"
        ? {label:"BLOCKED: delegate result (1 obligations)",tone:"error"}
        : {label:"WORKING (1 obligations)",tone:"working"})
    })
  }
}

test("a completion check waiting for the Vault unlock says what it waits for", () => {
  const evaluation = { origin: "completion_check" as const, prompt_id: "goal-check-p", work_prompt_id: "p", work_provider_run_id: null,
    objective_revision: 1, ledger_revision: 3, started_at_ms: 1, deadline_ms: 60001, result: null }
  const waiting = { ...task("working"), obligations: [], wait: null, reason: "Waiting for the Vault unlock",
    completion_check: { check_count: 1, evaluation: { ...evaluation, paused_at_ms: 5000 } } } as unknown as AgentTaskExecution
  const evaluating = { ...waiting, reason: "Evaluating original goal objective",
    completion_check: { check_count: 1, evaluation } } as unknown as AgentTaskExecution
  assert.deepEqual(sessionAgentTaskStatus({ agent_tasks: [waiting] } as RuntimeSession, "agent"),
    { label: "WORKING: Waiting for the Vault unlock", tone: "working" })
  assert.deepEqual(sessionAgentTaskStatus({ agent_tasks: [evaluating] } as RuntimeSession, "agent"),
    { label: "WORKING", tone: "working" })
})
