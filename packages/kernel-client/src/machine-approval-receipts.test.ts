import assert from "node:assert/strict"
import test from "node:test"

import { machineApprovalLinesForAgent, machineApprovalReceipts, withRuntimeInteractions, type MachineApprovalReceipt } from "./machine-approval-receipts.js"

const receipt = (state: MachineApprovalReceipt["state"], extra: Partial<MachineApprovalReceipt> = {}): MachineApprovalReceipt => ({
  id: "machine-acquisition-1", shown_in_agent_id: "agent-1", message: "builder in Build Room · Run the build — Acquire small at 0.000284 EUR per minute", state, expires_at_ms: null, ...extra,
})
const card = { id: "machine-acquisition-1", agent_id: "agent-1" }

test("an approved request stays visible while the service response is held, and closes when it is sent", () => {
  let session: { active_interactions?: unknown[]; machine_approval_receipts?: MachineApprovalReceipt[] } = {}
  const lines = () => machineApprovalLinesForAgent(session.machine_approval_receipts, "agent-1", 1_000)

  // The approval card is up: no receipt line yet.
  session = withRuntimeInteractions(session, [card], [])
  assert.deepEqual(lines(), [])
  // Approve: the card leaves and the receipt takes its place.
  session = withRuntimeInteractions(session, [], [receipt("approving")])
  assert.deepEqual(session.active_interactions, [])
  assert.deepEqual(lines(), ["builder in Build Room · Run the build — Acquire small at 0.000284 EUR per minute — Approving…"])
  // The service answered: the stripe closes.
  session = withRuntimeInteractions(session, [], [receipt("sent")])
  assert.deepEqual(lines(), [])
})

test("an expired approval shows its expiry time for ten minutes, to the agent that asked only", () => {
  const expired = receipt("expired", { expires_at_ms: Date.parse("2026-11-24T10:00:00Z") })
  const at = (offsetMs: number, agent = "agent-1") => machineApprovalLinesForAgent([expired], agent, Date.parse("2026-11-24T10:00:00Z") + offsetMs)
  assert.deepEqual(at(60_000), ["builder in Build Room · Run the build — Acquire small at 0.000284 EUR per minute — Expired at 2026-11-24T10:00:00Z"])
  assert.deepEqual(at(11 * 60_000), [])
  assert.deepEqual(at(60_000, "agent-2"), [])
  assert.deepEqual(machineApprovalLinesForAgent([expired], null), [])
})

test("malformed receipts from the kernel are dropped, not shown", () => {
  assert.deepEqual(machineApprovalReceipts(undefined), [])
  assert.deepEqual(machineApprovalReceipts([null, { id: 1 }, { id: "a", message: "m", state: "other" }, receipt("approving")]), [receipt("approving")])
})
