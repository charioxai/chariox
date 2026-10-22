// MP-08 / MP-09 / MP-10 / MP-11 A12: shared local/remote TUI goal visibility.
import assert from "node:assert/strict"
import test from "node:test"
import { sessionAgentTaskStatus } from "./session-runtime-status.js"
import type { RuntimeSession } from "./kernel-types-session.js"

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: long objectives cannot consume the terminal.
test("goal row keeps countdown and source count compact for long real requests", () => {
  const objective = "Review the current public sites and independently verify artifacts. ".repeat(50)
  const session = { agent_tasks: [{ task_id: "p", agent_id: "a", prompt_id: "p", state: "waiting",
    revision: 3, obligations: [], wait: { registration_ids: ["completion-" + "source-".repeat(50)] },
    goal: { id: "goal:p", objective, check_in_minutes: 1, active_elapsed_ms: 1000,
      waiting_elapsed_ms: 0, state_since_ms: 1000, next_check_in_ms: 61000, check_count: 1 },
  }] } as unknown as RuntimeSession
  const row = sessionAgentTaskStatus(session, "a", 2000)!
  assert(row.label.length < 600, "Goal footer must leave space for the transcript")
  assert.match(row.label.split("\n")[1]!, /^next check 59s/)
  assert.match(row.label, /sources 1/)
  assert.equal(session.agent_tasks![0]!.goal!.objective, objective, "display shortening cannot change the objective")
})

test("goal row exposes objective, evaluation, elapsed, check-in and revision controls", () => {
  const session = { agent_tasks: [{ task_id: "p", agent_id: "a", prompt_id: "check", pending_prompt_id: null, state: "working", revision: 3, reason: "", obligations: [], wait: null,
    goal: { id: "goal:p", objective: "Implement delegated work", objective_revision: 1, check_in_minutes: 1,
      active_elapsed_ms: 1000, waiting_elapsed_ms: 2000, check_count: 1, evaluation: { prompt_id: "check", result: null }, continuation_sequence: 0 } }] } as unknown as RuntimeSession
  const row = sessionAgentTaskStatus(session, "a")!
  assert.match(row.label, /GOAL.*EVALUATING.*Implement delegated work/)
  assert.match(row.label, /active 1s.*wait 2s/)
  assert.match(row.label, /1m.*check 1/)
  assert.match(row.label, /goal:p.*revision 3/)
})

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: client time does not change home phase.
test("goal waiting row shows a finite countdown without implying completion at zero", () => {
  const session={agent_tasks:[{task_id:"p",agent_id:"a",prompt_id:"p",state:"waiting",revision:3,obligations:[],wait:{registration_ids:["peer"]},goal:{id:"goal:p",objective:"Review live sites",check_in_minutes:1,active_elapsed_ms:1000,waiting_elapsed_ms:0,state_since_ms:1000,next_check_in_ms:61000,check_count:1}}]} as unknown as RuntimeSession
  assert.match(sessionAgentTaskStatus(session,"a",2000)!.label,/next check 59s/)
  assert.match(sessionAgentTaskStatus(session,"a",62000)!.label,/GOAL WAITING/)
  assert.match(sessionAgentTaskStatus(session,"a",62000)!.label,/next check 0s/)
})

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: active goals win; ordinary work supersedes history.
test("active goal survives peer work while terminal history yields to ordinary work", () => {
  const goal={task_id:"p",agent_id:"a",prompt_id:"p",state:"waiting",revision:3,obligations:[],wait:{registration_ids:["peer"]},goal:{id:"goal:p",objective:"Review live sites",check_in_minutes:1,active_elapsed_ms:1000,waiting_elapsed_ms:0,state_since_ms:1000,next_check_in_ms:61000,check_count:1}}
  const other={task_id:"message",agent_id:"a",prompt_id:"message",state:"working",revision:1,obligations:[{status:"open"}],wait:null}
  const session={agent_tasks:[goal,other]} as unknown as RuntimeSession
  assert.match(sessionAgentTaskStatus(session,"a",2000)!.label,/GOAL WAITING: Review live sites/)
  assert.match(sessionAgentTaskStatus(session,"a",2000)!.label,/0 obligations/)
  const done={agent_tasks:[{...goal,state:"done",wait:null},other]} as unknown as RuntimeSession
  assert.equal(sessionAgentTaskStatus(done,"a",2000)!.label, "WORKING (1 obligations)")
  const history={...done,agent_tasks:done.agent_tasks!.filter(task=>task.goal)}
  assert.match(sessionAgentTaskStatus(history,"a",2000)!.label,/GOAL DONE: Review live sites/)
})
