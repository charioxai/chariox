// MP-08 / MP-09 / MP-10 / MP-11 A12: shared local/remote TUI goal visibility.
import assert from "node:assert/strict"
import test from "node:test"
import { sessionAgentTaskStatus } from "./session-runtime-status.js"
import type { RuntimeSession } from "./kernel-types-session.js"
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
