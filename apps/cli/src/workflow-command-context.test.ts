import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
} from "./cli-types.js"
import { createWorkflowCommandContext } from "./workflow-command-context.js"

test("workflow command context resolves selected workflow fallbacks", () => {
  const context = createWorkflowCommandContext({
    selectedWorkflowId: () => "workflow-selected",
    sessionState: () => session(),
  })

  assert.equal(context.selectedWorkflowRef(), "workflow-selected")
  assert.equal(context.workflowRefOrSelected(undefined), "workflow-selected")
  assert.equal(context.workflowRefOrSelected("workflow-explicit"), "workflow-explicit")
})

test("workflow command context recognizes selected ids and session aliases", () => {
  const context = createWorkflowCommandContext({
    selectedWorkflowId: () => "workflow-selected",
    sessionState: () => session({
      workflows: [
        workflow({ id: "workflow-1", alias: "main" }),
        workflow({ id: "workflow-2" }),
      ],
    }),
  })

  assert.equal(context.isKnownWorkflowReference("workflow-selected"), true)
  assert.equal(context.isKnownWorkflowReference("workflow-1"), true)
  assert.equal(context.isKnownWorkflowReference("main"), true)
  assert.equal(context.isKnownWorkflowReference("missing"), false)
})

test("workflow command context marks first args explicit only when needed", () => {
  const withoutSelection = createWorkflowCommandContext({
    sessionState: () => session(),
  })
  const withSelection = createWorkflowCommandContext({
    selectedWorkflowId: () => "workflow-selected",
    sessionState: () => session({ workflows: [workflow({ id: "workflow-known" })] }),
  })

  assert.equal(withoutSelection.firstWorkflowArgIsExplicit("anything"), true)
  assert.equal(withSelection.firstWorkflowArgIsExplicit("workflow-known"), true)
  assert.equal(withSelection.firstWorkflowArgIsExplicit("node-1"), false)
})

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}
