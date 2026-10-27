import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowDefinition } from "./cli-types.js"
import { createWorkflowScreenController } from "./workflow-screen-controller.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"

test("workflow screen controller toggles the workspace screen while attached", () => {
  const harness = createHarness({ attached: true, screenMode: "agents" })

  harness.controller.toggleWorkspaceScreen()

  assert.equal(harness.screenMode, "workflow")
  assert.equal(harness.rebuilds, 1)
  assert.equal(harness.layouts, 1)
})

test("workflow screen controller ignores screen toggles while detached", () => {
  const harness = createHarness({ attached: false, screenMode: "agents" })

  harness.controller.toggleWorkspaceScreen()
  harness.controller.showWorkflowScreen()

  assert.equal(harness.screenMode, "agents")
  assert.equal(harness.rebuilds, 0)
  assert.equal(harness.layouts, 0)
})

test("workflow screen controller selects workflow canvases without layout churn", () => {
  const harness = createHarness({
    attached: true,
    screenMode: "workflow",
    selectedNodeId: "node-2",
  })

  harness.controller.selectWorkflowCanvas("workflow-2")

  assert.equal(harness.selectedWorkflowId, "workflow-2")
  assert.equal(harness.selectedNodeId, null)
  assert.equal(harness.rebuilds, 1)
  assert.equal(harness.layouts, 0)
})

test("workflow screen controller cycles selected nodes through the selected workflow", () => {
  const harness = createHarness({
    attached: true,
    screenMode: "workflow",
    selectedWorkflowId: "workflow-1",
    selectedNodeId: "node-1",
  })

  harness.controller.cycleWorkflowCanvasNode()

  assert.equal(harness.selectedNodeId, "node-2")
  assert.equal(harness.rebuilds, 1)
})

function createHarness(options: {
  attached: boolean
  screenMode: WorkspaceScreenMode
  selectedWorkflowId?: string | null
  selectedNodeId?: string | null
}) {
  const state: {
    attached: boolean
    screenMode: WorkspaceScreenMode
    selectedWorkflowId: string | null
    selectedNodeId: string | null
    rebuilds: number
    layouts: number
  } = {
    attached: options.attached,
    screenMode: options.screenMode,
    selectedWorkflowId: options.selectedWorkflowId ?? "workflow-1",
    selectedNodeId: options.selectedNodeId ?? null,
    rebuilds: 0,
    layouts: 0,
  }
  const controller = createWorkflowScreenController({
    isAttached: () => state.attached,
    workflows,
    selectedWorkflowId: () => state.selectedWorkflowId,
    setSelectedWorkflowId: (value) => {
      state.selectedWorkflowId = value
    },
    selectedWorkflowNodeId: () => state.selectedNodeId,
    setSelectedWorkflowNodeId: (value) => {
      state.selectedNodeId = value
    },
    workspaceScreenMode: () => state.screenMode,
    setWorkspaceScreenMode: (value) => {
      state.screenMode = value
    },
    rebuildTranscript: () => {
      state.rebuilds += 1
    },
    applyResponseLayout: () => {
      state.layouts += 1
    },
  })
  return {
    controller,
    get screenMode() { return state.screenMode },
    get selectedWorkflowId() { return state.selectedWorkflowId },
    get selectedNodeId() { return state.selectedNodeId },
    get rebuilds() { return state.rebuilds },
    get layouts() { return state.layouts },
  }
}

function workflows(): WorkflowDefinition[] {
  return [
    {
      id: "workflow-1",
      alias: "One",
      nodes: [
        { id: "node-1", agent_id: "agent-1" },
        { id: "node-2", agent_id: "agent-2" },
      ],
    },
    {
      id: "workflow-2",
      alias: "Two",
      nodes: [
        { id: "node-3", agent_id: "agent-3" },
      ],
    },
  ]
}
