import type { WorkspaceScreenMode } from "./workspace-screen.js"
import type { WorkflowInspectorMode } from "./workflow-inspector-projection.js"

export type WorkflowTerminalInspectorMode = WorkflowInspectorMode

type WorkflowTerminalPanelControllerDeps = {
  clearNodeInstructionsEditor: () => void
  setWorkflowInspectorMode: (mode: WorkflowTerminalInspectorMode) => void
  setSelectedWorkflowId: (workflowId: string) => void
  workflowScreenShowing: () => boolean
  setWorkspaceScreenMode: (mode: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
}

export type WorkflowTerminalPanelController = {
  open(workflowId: string, mode?: WorkflowInspectorMode): void
}

export function createWorkflowTerminalPanelController(
  deps: WorkflowTerminalPanelControllerDeps,
): WorkflowTerminalPanelController {
  return {
    open(workflowId, mode = "logs") {
      deps.clearNodeInstructionsEditor()
      deps.setWorkflowInspectorMode(mode)
      deps.setSelectedWorkflowId(workflowId)
      if (!deps.workflowScreenShowing()) {
        deps.setWorkspaceScreenMode("workflow")
      }
      deps.rebuildTranscript()
    },
  }
}
