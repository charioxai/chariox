import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeSession,
} from "./cli-types.js"
import {
  createWorkflowInspectorController,
} from "./workflow-inspector-controller.js"
import type {
  WorkflowInspectorMode,
} from "./workflow-inspector-projection.js"
import type {
  WorkflowNodeInstructionsEditor,
} from "./workflow-node-instructions-editor-controller.js"

test("workflow inspector controller projects current runtime selection", () => {
  let selectedNodeId: string | null = "node-a"
  const controller = createWorkflowInspectorController({
    getSession: () => session(),
    getSelectedWorkflowId: () => "workflow-a",
    getSelectedWorkflowNodeId: () => selectedNodeId,
    getInspectorMode: () => "runtime",
    getNodeInstructionsEditor: () => null,
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(controller.project()?.title, "Workflow Runtime")
  assert.equal(controller.project()?.meta.includes("Selected node: node-a"), true)

  selectedNodeId = null

  assert.equal(controller.project()?.meta.includes("Selected node: -"), true)
})

test("workflow inspector controller wires node-instructions editor callbacks", () => {
  let draft = "existing"
  let mode: WorkflowInspectorMode = "terminal"
  let inputRefSet = false
  let editor: WorkflowNodeInstructionsEditor | null = {
    workflowId: "workflow-a",
    nodeId: "node-a",
    draft,
  }
  const controller = createWorkflowInspectorController({
    getSession: () => session(),
    getSelectedWorkflowId: () => "workflow-a",
    getSelectedWorkflowNodeId: () => "node-a",
    getInspectorMode: () => mode,
    getNodeInstructionsEditor: () => editor,
    updateNodeInstructionsDraft: (nextDraft) => {
      draft = nextDraft
    },
    setNodeInstructionsInputRef: () => {
      inputRefSet = true
    },
  })

  const projection = controller.project()

  assert.equal(projection?.title, "Node Instructions")
  assert.equal(projection?.draft, "existing")

  projection?.onDraftChange?.("next")
  projection?.onEditorRef?.(null)

  assert.equal(draft, "next")
  assert.equal(inputRefSet, true)

  editor = null

  assert.equal(controller.project()?.title, "Workflow Terminal")
})

function session(): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 1,
    agents: [agent("agent-a")],
    workflows: [
      {
        id: "workflow-a",
        alias: null,
        nodes: [
          {
            id: "node-a",
            agent_id: "agent-a",
          },
        ],
        edges: [],
        endpoints: [],
      },
    ],
    config_state: { values: {} } as RuntimeSession["config_state"],
  }
}

function agent(id: string): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: null,
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
