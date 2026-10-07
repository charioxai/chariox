import assert from "node:assert/strict"
import test from "node:test"
import { controlRoomWorkflowRunsRequest, roomWorkflowRows, roomWorkflowRunTargets, ROOM_WORKFLOWS_MIN_PROTOCOL } from "./room-workflows.js"
import type { RoomWorkflowInventory } from "./kernel-types.js"

const inventory = {
  session_id: "room", home_kernel_id: "home", revision: "one", workflow_count: 2,
  workflows: [
    { workflow_id: "first", workflow_revision: 1, label: "First", state: "mixed", running_count: 1, paused_count: 1, queued_count: 0,
      endpoints: ["a", "b"].map(endpoint_id => ({ endpoint_id, entry_node_id: "node", label: endpoint_id, can_start: true })),
      runs: [
        { run_id: "running", endpoint_id: "a", status: "Completing", can_pause: true, can_resume: false, can_stop: true },
        { run_id: "paused", endpoint_id: "b", status: "Paused", can_pause: false, can_resume: true, can_stop: true },
      ] },
    { workflow_id: "blank", workflow_revision: 0, label: "Blank", state: "idle", running_count: 0, paused_count: 0, queued_count: 0, endpoints: [], runs: [] },
  ],
} satisfies RoomWorkflowInventory

test("every endpoint is one manual row, including several endpoints per workflow", () => {
  assert.equal(ROOM_WORKFLOWS_MIN_PROTOCOL, 436)
  const rows = roomWorkflowRows(inventory)
  assert.equal(rows.length, 2)
  assert.deepEqual(rows.map(row => row.endpoint.endpoint_id), ["a", "b"])
  assert.equal(rows[0]?.workflow.state, "mixed")
})

test("controls capture all eligible current runs and keep the captured ids", () => {
  const ids = roomWorkflowRunTargets(inventory.workflows[0]!, "stop")
  const request = controlRoomWorkflowRunsRequest("room", "first", "stop", ids)
  ids.push("later")
  assert.deepEqual(request, { ControlRoomWorkflowRuns: { session_id: "room", workflow_id: "first", action: "stop", run_ids: ["running", "paused"] } })
  assert.deepEqual(roomWorkflowRunTargets(inventory.workflows[0]!, "pause"), ["running"])
  assert.deepEqual(roomWorkflowRunTargets(inventory.workflows[0]!, "resume"), ["paused"])
})
