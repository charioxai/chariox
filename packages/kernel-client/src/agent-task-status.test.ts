// MP-08 / MP-09 / MP-10 / MP-11 A02, supplementary projection regressions.
import assert from "node:assert/strict"
import test from "node:test"
import type { AgentTaskExecution, RuntimeSession } from "./kernel-types-session.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { sessionAgentTaskStatus } from "./session-runtime-status.js"
function task(state: AgentTaskExecution["state"]): AgentTaskExecution {
  return { task_id: state, room_id: "room", owner_user_id: "owner", agent_id: "agent", prompt_id: state,
    provider_run_id: null, revision: 1, blocked_revision: 0, state, reason: "delegate result",
    obligations: [{ id: "child", kind: "delegate", resource_id: "child", completion_task_id: null, status: "open", dispatch_state: "accepted" }],
    wait: { registration_ids: ["reg"], deadline_ms: 60000, started_at_ms: 0, inbox_cursor: 0, long_wait_notified: false, last_checked_at_ms: 0 },
    last_progress_at_ms: 0, progress_sequence: 0, no_progress_wakes: 0, correction_used: false, pending_prompt_id: null }
}
test("MP-08/MP-10/MP-11 A02 newer done cannot hide older waiting task", () => {
  const session = { agent_tasks: [task("waiting"), { ...task("done"), obligations: [] }] } as RuntimeSession
  assert.match(sessionAgentTaskStatus(session, "agent")!.label, /^WAITING ON delegate result UNTIL .*\(1 obligations\)$/)
})
test("MP-08/MP-10/MP-11 A02 blocked takes priority and retained obligations remain visible", () => {
  const session = { agent_tasks: [task("waiting"), task("blocked")] } as RuntimeSession
  assert.deepEqual(sessionAgentTaskStatus(session, "agent"), { label: "BLOCKED: delegate result (2 obligations)", tone: "error" })
  assert.equal(sessionAgentTaskStatus(session, "foreign"), null)
})

test("MP-08/MP-10/MP-11 A02 task DTO requires protocol485",()=>{ assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 485) })


test("MP-08/MP-09/MP-10/MP-11 A02 malformed wait is safely blocked", () => {
  const session = { agent_tasks: [{ ...task("waiting"), wait: null }] } as RuntimeSession
  assert.match(sessionAgentTaskStatus(session, "agent")!.label, /^BLOCKED: Invalid wait deadline/)
})
