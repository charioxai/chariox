import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import {
  deriveWorkflowPromptState,
  formatWorkflowPromptPlaceholder,
  isWorkflowCommandInput,
  resolveActiveWorkflowRun,
  validateWorkflowPromptSubmit,
} from "./workflow-prompt-state.js"

test("resolveActiveWorkflowRun returns the newest non-terminal run", () => {
  const active = resolveActiveWorkflowRun("workflow-1", [
    workflowRun({ id: "run-1", created_at_ms: 1, status: "Completed" }),
    workflowRun({ id: "run-2", created_at_ms: 2, status: "Running" }),
    workflowRun({ id: "run-3", created_at_ms: 3, status: "Queued" }),
  ])

  assert.equal(active?.id, "run-3")
})

test("deriveWorkflowPromptState targets the selected workflow node agent", () => {
  assert.equal(
    deriveWorkflowPromptState({
      workflowScreenActive: true,
      workflows: [workflow({ nodes: [] })],
      workflowRuns: [],
      agents: agents(),
      selectedWorkflowId: "workflow-1",
      selectedWorkflowNodeId: "node-1",
    }).disabledReason,
    "no workflow node selected",
  )

  assert.equal(
    deriveWorkflowPromptState({
      workflowScreenActive: true,
      workflows: [workflow()],
      workflowRuns: [],
      agents: [agent("agent-2")],
      selectedWorkflowId: "workflow-1",
      selectedWorkflowNodeId: "node-1",
    }).disabledReason,
    "selected node agent unavailable",
  )

  const eligible = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow()],
    workflowRuns: [],
    agents: agents(),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
  })

  assert.equal(eligible.enabled, true)
  assert.equal(eligible.selectedAgent?.id, "agent-1")
})

test("formatWorkflowPromptPlaceholder reflects workflow eligibility", () => {
  const enabledState = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow()],
    workflowRuns: [],
    agents: agents(),
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
    "Prompt workflow agent agent-1 (Builder)",
  )

  const disabledState = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow({ nodes: [] })],
    workflowRuns: [],
    agents: agents(),
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
    "Workflow prompt disabled: no workflow node selected. Use /workflow ...",
  )
})

test("isWorkflowCommandInput requires slash as the first character", () => {
  assert.equal(isWorkflowCommandInput("/workflow run"), true)
  assert.equal(isWorkflowCommandInput(" /workflow run"), false)
  assert.equal(isWorkflowCommandInput("hello"), false)
})

test("validateWorkflowPromptSubmit returns footer decisions", () => {
  const enabledState = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow()],
    workflowRuns: [],
    agents: agents(),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
  })

  assert.deepEqual(validateWorkflowPromptSubmit({
    state: enabledState,
    pendingAttachmentCount: 0,
  }), {
    ok: true,
    targetAgentId: "agent-1",
  })

  const disabledState = deriveWorkflowPromptState({
    workflowScreenActive: true,
    workflows: [workflow({ nodes: [] })],
    workflowRuns: [],
    agents: agents(),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
  })
  assert.deepEqual(validateWorkflowPromptSubmit({
    state: disabledState,
    pendingAttachmentCount: 0,
  }), {
    ok: false,
    message: "prompt disabled: no workflow node selected",
    tone: "info",
  })
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

function agents(): AgentInstance[] {
  return [agent("agent-1", "Builder"), agent("agent-2", "Reviewer")]
}

function agent(id: string, alias: string | null = null): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias,
    provider: "codex",
    model: "gpt-5",
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
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
