// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: RPC/event races preserve home revisions.
import assert from "node:assert/strict"
import test from "node:test"
import { withAgentTaskUpdate } from "./agent-task-state.js"
import type { AgentTaskExecution, RuntimeSession } from "./kernel-types-session.js"

test("compact tasks preserve the room and reject replay, foreign room and unknown agent", () => {
  const task={task_id:"task",room_id:"room",agent_id:"agent",revision:7,state:"working"} as AgentTaskExecution
  const session={id:"room",agents:[{id:"agent"}],alias:"Owner room",agent_tasks:[task],prompt_states:{retained:{}}} as unknown as RuntimeSession
  const update={...task,revision:8,state:"waiting"} as AgentTaskExecution
  const next=withAgentTaskUpdate(session,"room",update)
  assert.equal(next.alias,"Owner room")
  assert.equal(next.agents,session.agents)
  assert.equal(next.prompt_states,session.prompt_states)
  assert.deepEqual(next.agent_tasks,[update])
  assert.equal(withAgentTaskUpdate(next,"room",task),next)
  assert.equal(withAgentTaskUpdate(next,"foreign",{...update,revision:9}),next)
  assert.equal(withAgentTaskUpdate(next,"room",{...update,revision:9,room_id:"foreign"}),next)
  assert.equal(withAgentTaskUpdate(next,"room",{...update,revision:9,agent_id:"foreign"}),next)
  assert.equal(withAgentTaskUpdate(next,"room",{...update,revision:NaN}),next)
  assert.equal(withAgentTaskUpdate(next,"room",{...update,revision:9,task_id:""}),next)
  assert.equal(session.agent_tasks![0]!.revision,7)
})
