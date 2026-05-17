import type { WorkspaceScreenMode } from "./workspace-screen.js"

export type WorkflowTerminalInspectorMode = "terminal"

type WorkflowTerminalPanelControllerDeps = {
  clearNodeInstructionsEditor: () => void
  setWorkflowInspectorMode: (mode: WorkflowTerminalInspectorMode) => void
  setSelectedWorkflowId: (workflowId: string) => void
  workflowScreenShowing: () => boolean
  setWorkspaceScreenMode: (mode: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
}

export type WorkflowTerminalPanelController = {
  open(workflowId: string): void
}

export function createWorkflowTerminalPanelController(
  deps: WorkflowTerminalPanelControllerDeps,
): WorkflowTerminalPanelController {
  return {
    open(workflowId) {
      deps.clearNodeInstructionsEditor()
      deps.setWorkflowInspectorMode("terminal")
      deps.setSelectedWorkflowId(workflowId)
      if (!deps.workflowScreenShowing()) {
        deps.setWorkspaceScreenMode("workflow")
      }
      deps.rebuildTranscript()
    },
  }
}
