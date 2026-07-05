import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowRun } from "./cli-types.js"
import { createWorkflowPromptSubmitController } from "./workflow-prompt-submit-controller.js"
import type { WorkflowPromptState } from "@arroba/kernel-client/workflow-prompt-state"

test("workflow prompt submit reports disabled prompt state", async () => {
  const harness = createHarness({
    workflowPromptState: {
      ...enabledWorkflowPromptState(),
      enabled: false,
      disabledReason: "no active workflow run",
    },
  })

  await harness.controller.submit("hello")

  assert.equal(harness.footerMessages().at(-1)?.message, "prompt disabled: no active workflow run")
  assert.deepEqual(harness.agentPrompts(), [])
})

test("workflow prompt submit targets the selected backing agent", async () => {
  const harness = createHarness()

  await harness.controller.submit("hello")

  assert.deepEqual(harness.agentPrompts(), [{ rawPrompt: "hello", targetAgentId: "agent-1" }])
})

test("workflow prompt submit allows attachments through the normal agent prompt path", async () => {
  const harness = createHarness({ pendingAttachmentCount: 1 })

  await harness.controller.submit("hello\n")

  assert.deepEqual(harness.agentPrompts(), [{ rawPrompt: "hello\n", targetAgentId: "agent-1" }])
})

test("workflow prompt submit reports agent prompt failure", async () => {
  const harness = createHarness({
    submitAgentPrompt: async () => {
      throw new Error("agent unavailable")
    },
  })

  await harness.controller.submit("hello")

  assert.equal(harness.footerMessages().at(-1)?.message, "agent unavailable")
})

function createHarness(options: {
  workflowPromptState?: WorkflowPromptState
  pendingAttachmentCount?: number
  submitAgentPrompt?: (rawPrompt: string, targetAgentId: string) => Promise<void>
} = {}) {
  const agentPrompts: Array<{ rawPrompt: string; targetAgentId: string }> = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []

  const controller = createWorkflowPromptSubmitController({
    getWorkflowPromptState: () => options.workflowPromptState ?? enabledWorkflowPromptState(),
    getPendingAttachmentCount: () => options.pendingAttachmentCount ?? 0,
    submitAgentPrompt: async (rawPrompt, targetAgentId) => {
      agentPrompts.push({ rawPrompt, targetAgentId })
      await options.submitAgentPrompt?.(rawPrompt, targetAgentId)
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    agentPrompts: () => agentPrompts,
    footerMessages: () => footerMessages,
  }
}

function enabledWorkflowPromptState(): WorkflowPromptState {
  return {
    workflow: {
      id: "workflow-1",
      alias: null,
      nodes: [],
      edges: [],
      endpoints: [{ id: "endpoint-1", alias: null, entry_node_id: "node-1" }],
    },
    workflowRun: workflowRun("run-active", "Running"),
    selectedNodeId: "node-1",
    selectedAgent: {
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: "session-1",
      alias: null,
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
    },
    enabled: true,
    disabledReason: null,
  }
}

function workflowRun(id: string, status: string): WorkflowRun {
  return {
    id,
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status,
    invocation_prompt: "hello",
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 1,
    started_at_ms: 1,
    completed_at_ms: null,
  }
}
