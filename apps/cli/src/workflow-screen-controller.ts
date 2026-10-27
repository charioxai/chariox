import type { WorkflowDefinition } from "./cli-types.js"
import { toggleWorkspaceScreenMode, type WorkspaceScreenMode } from "./workspace-screen.js"
import {
  cycleWorkflowNodeId,
  resolveSelectedWorkflow,
} from "./workflow-graph/index.js"
import type { WorkflowInspectorMode } from "./workflow-inspector-projection.js"
import type { WorkflowComponentSelection } from "./workflow-component-selection.js"

type WorkflowScreenControllerDeps = {
  isAttached: () => boolean
  workflows: () => WorkflowDefinition[]
  selectedWorkflowId: () => string | null
  setSelectedWorkflowId: (value: string | null) => void
  selectedWorkflowNodeId: () => string | null
  setSelectedWorkflowNodeId: (value: string | null) => void
  setSelectedWorkflowComponent?: (value: WorkflowComponentSelection | null) => void
  setWorkflowInspectorMode?: (value: WorkflowInspectorMode) => void
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
    deps.setSelectedWorkflowComponent?.({ kind: "workflow" })
    deps.setWorkflowInspectorMode?.("logs")
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
    deps.setSelectedWorkflowComponent?.(nextNodeId ? { kind: "node", id: nextNodeId } : { kind: "workflow" })
    deps.setWorkflowInspectorMode?.(nextNodeId ? "trace" : "logs")
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
