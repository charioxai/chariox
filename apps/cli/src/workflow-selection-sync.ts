import type { WorkflowDefinition } from "./cli-types.js"
import {
  resolveSelectedWorkflow,
  resolveSelectedWorkflowNodeId,
} from "./workflow-graph/index.js"

type WorkflowSelectionSyncControllerDeps = {
  workflows: () => WorkflowDefinition[]
  selectedWorkflowId: () => string | null
  selectedWorkflowNodeId: () => string | null
  setSelectedWorkflowId: (value: string | null) => void
  setSelectedWorkflowNodeId: (value: string | null) => void
}

export function deriveWorkflowSelectionState(
  workflows: WorkflowDefinition[],
  selectedWorkflowId: string | null,
  selectedNodeId: string | null,
) {
  const workflow = resolveSelectedWorkflow(workflows, selectedWorkflowId)
  return {
    workflow,
    workflowId: workflow?.id ?? null,
    nodeId: resolveSelectedWorkflowNodeId(workflow, selectedNodeId),
  }
}

export function createWorkflowSelectionSyncController(
  deps: WorkflowSelectionSyncControllerDeps,
) {
  return {
    sync() {
      const nextSelection = deriveWorkflowSelectionState(
        deps.workflows(),
        deps.selectedWorkflowId(),
        deps.selectedWorkflowNodeId(),
      )
      if (deps.selectedWorkflowId() !== nextSelection.workflowId) {
        deps.setSelectedWorkflowId(nextSelection.workflowId)
      }
      if (deps.selectedWorkflowNodeId() !== nextSelection.nodeId) {
        deps.setSelectedWorkflowNodeId(nextSelection.nodeId)
      }
    },
  }
}
