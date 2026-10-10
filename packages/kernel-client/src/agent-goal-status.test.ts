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
