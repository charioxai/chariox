import assert from "node:assert/strict"
import test from "node:test"
import type { AgentRuntimeActivity, TerminalOutputRecord } from "../cli-types.js"
import { createNativeTurnSettlementBarrier } from "./native-turn-settlement.js"

test("MP-08 MP-10 a restarted terminal stream cannot release a new completion using old delivered IDs", () => {
  const barrier = createNativeTurnSettlementBarrier()
  const activity = (promptId: string) => ({ agent_id: "agent", last_completed_turn: {
    agent_id: "agent", prompt_id: promptId, provider_run_id: "run", completed_at_ms: 1,
    settlement_status: "completed",
  } }) as unknown as AgentRuntimeActivity
  const records = (recordId: number) => [{ record_id: recordId }] as TerminalOutputRecord[]
  barrier.observe({ agent: activity("before-restart") }, 100)
  barrier.projectRecords(records(99))
  assert.equal(barrier.release(false).length, 1)
  barrier.observe({ agent: activity("after-restart") }, 2)
  barrier.projectRecords(records(0))
  assert.deepEqual(barrier.release(false), [])
  barrier.projectRecords(records(1))
  assert.equal(barrier.release(false)[0]?.agent?.last_completed_turn?.prompt_id, "after-restart")
})
