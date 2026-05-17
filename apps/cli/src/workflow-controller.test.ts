import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowDefinition } from "./cli-types.js"
import {
  createWorkflowSelectionSyncController,
  deriveWorkflowSelectionState,
} from "./workflow-controller.js"

test("deriveWorkflowSelectionState keeps a valid workflow and node selection", () => {
  const next = deriveWorkflowSelectionState(workflows(), "wf-2", "wf-2-node-1")

  assert.equal(next.workflow?.id, "wf-2")
  assert.equal(next.workflowId, "wf-2")
  assert.equal(next.nodeId, "wf-2-node-1")
})

test("workflow selection sync repairs stale workflow and node ids", () => {
  const calls: string[] = []
  let selectedWorkflowId: string | null = "missing"
  let selectedWorkflowNodeId: string | null = "missing-node"
  const controller = createWorkflowSelectionSyncController({
    workflows,
    selectedWorkflowId: () => selectedWorkflowId,
    selectedWorkflowNodeId: () => selectedWorkflowNodeId,
    setSelectedWorkflowId: (value) => {
      selectedWorkflowId = value
      calls.push(`workflow:${value ?? "null"}`)
    },
    setSelectedWorkflowNodeId: (value) => {
      selectedWorkflowNodeId = value
      calls.push(`node:${value ?? "null"}`)
    },
  })

  controller.sync()

  assert.equal(selectedWorkflowId, "wf-1")
  assert.equal(selectedWorkflowNodeId, "wf-1-node-1")
  assert.deepEqual(calls, ["workflow:wf-1", "node:wf-1-node-1"])
})

test("workflow selection sync is idle when the selection is already valid", () => {
  const calls: string[] = []
  const controller = createWorkflowSelectionSyncController({
    workflows,
    selectedWorkflowId: () => "wf-2",
    selectedWorkflowNodeId: () => "wf-2-node-1",
    setSelectedWorkflowId: (value) => {
      calls.push(`workflow:${value ?? "null"}`)
    },
    setSelectedWorkflowNodeId: (value) => {
      calls.push(`node:${value ?? "null"}`)
    },
  })

  controller.sync()

  assert.deepEqual(calls, [])
})

function workflows(): WorkflowDefinition[] {
  return [
    {
      id: "wf-1",
      alias: "One",
      nodes: [
        { id: "wf-1-node-1", agent_id: "agent-1" },
      ],
    },
    {
      id: "wf-2",
      alias: "Two",
      nodes: [
        { id: "wf-2-node-1", agent_id: "agent-2" },
      ],
    },
  ]
}
