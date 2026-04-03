import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import {
  deriveWorkflowPromptState,
  formatWorkflowPromptPlaceholder,
  isWorkflowCommandInput,
  resolveActiveWorkflowRun,
} from "./workflow-prompt-state.js"

test("resolveActiveWorkflowRun returns the newest non-terminal run", () => {
  const active = resolveActiveWorkflowRun("workflow-1", [
    workflowRun({ id: "run-1", created_at_ms: 1, status: "Completed" }),
    workflowRun({ id: "run-2", created_at_ms: 2, status: "Running" }),
    workflowRun({ id: "run-3", created_at_ms: 3, status: "Queued" }),
  ])

  assert.equal(active?.id, "run-3")
})

test("deriveWorkflowPromptState requires endpoints, active run, and selected endpoint node", () => {
  const baseWorkflow = workflow({
    endpoints: [{ id: "endpoint-1", alias: "start", entry_node_id: "node-1" }],
  })

  assert.equal(
    deriveWorkflowPromptState({
      workflowScreenActive: true,
      workflows: [workflow({ endpoints: [] })],
      workflowRuns: [],
      selectedWorkflowId: "workflow-1",
      selectedWorkflowNodeId: "node-1",
    }).disabledReason,
    "no workflow endpoints configured",
  )

  assert.equal(
    deriveWorkflowPromptState({
      workflowScreenActive: true,
      workflows: [baseWorkflow],
      workflowRuns: [],
      selectedWorkflowId: "workflow-1",
      selectedWorkflowNodeId: "node-1",
    }).disabledReason,
    "no active workflow run",
  )

  assert.equal(
    deriveWorkflowPromptState({
      workflowScreenActive: true,
      workflows: [baseWorkflow],
      workflowRuns: [workflowRun({ id: "run-1", status: "Running" })],
      selectedWorkflowId: "workflow-1",
      selectedWorkflowNodeId: "node-2",
    }).disabledReason,
    "selected node has no endpoint",
  )

  const eligible = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [baseWorkflow],
    workflowRuns: [workflowRun({ id: "run-1", status: "Running" })],
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
  })

  assert.equal(eligible.enabled, true)
  assert.equal(eligible.endpoint?.id, "endpoint-1")
})

test("formatWorkflowPromptPlaceholder reflects workflow eligibility", () => {
  const enabledState = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow({
      endpoints: [{ id: "endpoint-1", alias: "start", entry_node_id: "node-1" }],
    })],
    workflowRuns: [workflowRun({ id: "run-1", status: "Running" })],
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
  })
  assert.equal(
    formatWorkflowPromptPlaceholder({
      workflowScreenActive: true,
      state: enabledState,
      attachedPlaceholder: "Write your next prompt here",
      detachedPlaceholder: "Start a session",
    }),
    "Send prompt to endpoint endpoint-1 (start)",
  )

  const disabledState = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow({ endpoints: [] })],
    workflowRuns: [],
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
  })
  assert.equal(
    formatWorkflowPromptPlaceholder({
      workflowScreenActive: true,
      state: disabledState,
      attachedPlaceholder: "Write your next prompt here",
      detachedPlaceholder: "Start a session",
    }),
    "Workflow prompt disabled: no workflow endpoints configured. Use /workflow ...",
  )
})

test("isWorkflowCommandInput requires slash as the first character", () => {
  assert.equal(isWorkflowCommandInput("/workflow run"), true)
  assert.equal(isWorkflowCommandInput(" /workflow run"), false)
  assert.equal(isWorkflowCommandInput("hello"), false)
})

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [
      { id: "node-1", agent_id: "agent-1" },
      { id: "node-2", agent_id: "agent-2" },
    ],
    edges: [],
    endpoints: [],
    ...overrides,
  }
}

function workflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status: "Running",
    invocation_prompt: null,
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 1,
    started_at_ms: 1,
    completed_at_ms: null,
    ...overrides,
  }
}
