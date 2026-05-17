import type { WorkflowDefinition } from "./cli-types.js"
import { toggleWorkspaceScreenMode, type WorkspaceScreenMode } from "./workspace-screen.js"
import {
  cycleWorkflowNodeId,
  resolveSelectedWorkflow,
} from "./workflow-graph/index.js"

type WorkflowScreenControllerDeps = {
  isAttached: () => boolean
  workflows: () => WorkflowDefinition[]
  selectedWorkflowId: () => string | null
  setSelectedWorkflowId: (value: string | null) => void
  selectedWorkflowNodeId: () => string | null
  setSelectedWorkflowNodeId: (value: string | null) => void
  workspaceScreenMode: () => WorkspaceScreenMode
  setWorkspaceScreenMode: (value: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
}

export function createWorkflowScreenController(deps: WorkflowScreenControllerDeps) {
  const workflowScreenActive = () => deps.isAttached() && deps.workspaceScreenMode() === "workflow"
  const selectedWorkflow = () => resolveSelectedWorkflow(deps.workflows(), deps.selectedWorkflowId())

  const toggleWorkspaceScreen = () => {
    if (!deps.isAttached()) {
      return
    }
    deps.setWorkspaceScreenMode(toggleWorkspaceScreenMode(deps.workspaceScreenMode()))
    deps.rebuildTranscript()
    deps.applyResponseLayout()
  }

  const showWorkflowScreen = () => {
    if (!deps.isAttached() || workflowScreenActive()) {
      return
    }
    deps.setWorkspaceScreenMode("workflow")
    deps.rebuildTranscript()
    deps.applyResponseLayout()
  }

  const selectWorkflowCanvas = (workflowId: string | null) => {
    deps.setSelectedWorkflowId(workflowId)
    deps.setSelectedWorkflowNodeId(null)
    if (workflowScreenActive()) {
      deps.rebuildTranscript()
    }
  }

  const cycleWorkflowCanvasNode = (step = 1) => {
    const nextNodeId = cycleWorkflowNodeId(selectedWorkflow(), deps.selectedWorkflowNodeId(), step)
    if (nextNodeId === deps.selectedWorkflowNodeId()) {
      return
    }
    deps.setSelectedWorkflowNodeId(nextNodeId)
    if (workflowScreenActive()) {
      deps.rebuildTranscript()
    }
  }

  return {
    workflowScreenActive,
    selectedWorkflow,
    toggleWorkspaceScreen,
    showWorkflowScreen,
    selectWorkflowCanvas,
    cycleWorkflowCanvasNode,
  }
}
