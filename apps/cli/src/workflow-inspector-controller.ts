import type { TextareaRenderable } from "@opentui/core"

import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  buildWorkflowInspectorProjection,
  type WorkflowInspectorMode,
  type WorkflowInspectorProjection,
} from "./workflow-inspector-projection.js"
import type {
  WorkflowNodeInstructionsEditor,
} from "./workflow-node-instructions-editor-controller.js"

type WorkflowInspectorControllerDeps = {
  getSession: () => RuntimeSession
  getSelectedWorkflowId: () => string | null
  getSelectedWorkflowNodeId: () => string | null
  getInspectorMode: () => WorkflowInspectorMode
  getNodeInstructionsEditor: () => WorkflowNodeInstructionsEditor | null
  getAgentPaneEntries: () => Record<string, TranscriptEntry[]>
  updateNodeInstructionsDraft: (draft: string) => void
  setNodeInstructionsInputRef: (editor: TextareaRenderable | null) => void
}

export function createWorkflowInspectorController(
  deps: WorkflowInspectorControllerDeps,
): {
  project: () => WorkflowInspectorProjection | null
} {
  const project = (): WorkflowInspectorProjection | null => {
    return buildWorkflowInspectorProjection({
      session: deps.getSession(),
      selectedWorkflowId: deps.getSelectedWorkflowId(),
      selectedWorkflowNodeId: deps.getSelectedWorkflowNodeId(),
      inspectorMode: deps.getInspectorMode(),
      nodeInstructionsEditor: deps.getNodeInstructionsEditor(),
      agentPaneEntries: deps.getAgentPaneEntries(),
      updateNodeInstructionsDraft: deps.updateNodeInstructionsDraft,
      setNodeInstructionsInputRef: deps.setNodeInstructionsInputRef,
    })
  }

  return {
    project,
  }
}
