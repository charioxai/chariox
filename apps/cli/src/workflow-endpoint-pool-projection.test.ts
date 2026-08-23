import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowEndpointRuntimeInstance, WorkflowRun } from "./cli-types.js"
import {
  buildWorkflowEndpointPoolStatus,
  formatWorkflowEndpointPoolSummary,
  parseWorkflowEndpointMaxInstances,
  workflowEndpointCapacity,
} from "./workflow-endpoint-pool-projection.js"

test("endpoint capacity falls back to the kernel default of one", () => {
  assert.equal(workflowEndpointCapacity({ id: "endpoint-1" }), 1)
  assert.equal(workflowEndpointCapacity({ id: "endpoint-1", max_instances: 4 }), 4)
})

test("parseWorkflowEndpointMaxInstances accepts only counts within the kernel limit", () => {
  assert.equal(parseWorkflowEndpointMaxInstances("1"), 1)
  assert.equal(parseWorkflowEndpointMaxInstances("32"), 32)
  assert.equal(parseWorkflowEndpointMaxInstances("0"), null)
  assert.equal(parseWorkflowEndpointMaxInstances("33"), null)
  assert.equal(parseWorkflowEndpointMaxInstances("none"), null)
  assert.equal(parseWorkflowEndpointMaxInstances(""), null)
})

test("pool status summarizes live endpoint instances and active runs", () => {
  const pool = buildWorkflowEndpointPoolStatus(
    "workflow-1",
    { id: "endpoint-1", max_instances: 3 },
    [
      instance({ id: "instance-1", status: "busy", active_run_id: "run-1" }),
      instance({ id: "instance-2", status: "idle" }),
      instance({ id: "instance-x", endpoint_id: "endpoint-2", status: "busy" }),
      instance({ id: "instance-y", workflow_id: "workflow-2", status: "busy" }),
    ],
    [
      run("run-2", "Running"),
      run("run-9", "Completed"),
      run("run-other-workflow", "Running", "workflow-2"),
    ],
  )

  assert.deepEqual(pool, {
    capacity: 3,
    registered: 2,
    busyCount: 1,
    staleCount: 0,
    activeRunIds: ["run-1", "run-2"],
  })
  assert.equal(
    formatWorkflowEndpointPoolSummary(pool),
    "1/3 busy • 2 registered • 2 active runs",
  )
})

test("pool status reports stale instances and singular run grammar", () => {
  const pool = buildWorkflowEndpointPoolStatus(
    "workflow-1",
    { id: "endpoint-1" },
    [instance({ id: "instance-1", status: "stale" })],
    [],
  )

  assert.equal(pool.capacity, 1)
  assert.equal(pool.staleCount, 1)
  assert.equal(pool.activeRunIds.length, 0)
  assert.equal(
    formatWorkflowEndpointPoolSummary(pool),
    "0/1 busy • 1 registered • 1 stale • 0 active runs",
  )
})

function instance(overrides: Partial<WorkflowEndpointRuntimeInstance> = {}): WorkflowEndpointRuntimeInstance {
  return {
    id: "instance-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    workflow_revision: 1,
    ordinal: 1,
    primary: true,
    node_agent_ids: {},
    worktree_id: "worktree-1",
    status: "idle",
    active_run_id: null,
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}

function run(id: string, status: string, workflowId = "workflow-1"): WorkflowRun {
  return {
    id,
    workflow_id: workflowId,
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status,
    invocation_prompt: null,
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 10,
    started_at_ms: null,
    completed_at_ms: null,
  }
}
