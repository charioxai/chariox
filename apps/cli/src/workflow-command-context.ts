import type {
  RuntimeSession,
} from "./cli-types.js"

export type WorkflowCommandContextDeps = {
  selectedWorkflowId?: () => string | null
  sessionState: () => RuntimeSession
}

export type WorkflowCommandContext = ReturnType<typeof createWorkflowCommandContext>

export function createWorkflowCommandContext(deps: WorkflowCommandContextDeps) {
  const selectedWorkflowRef = () => deps.selectedWorkflowId?.() ?? null
  const workflowRefOrSelected = (workflowRef: string | null | undefined) => workflowRef ?? selectedWorkflowRef()
  const isKnownWorkflowReference = (reference: string | undefined) => {
    if (!reference) {
      return false
    }
    if (reference === selectedWorkflowRef()) {
      return true
    }
    return (deps.sessionState().workflows ?? []).some((workflow) => (
      workflow.id === reference || workflow.alias === reference
    ))
  }
  const firstWorkflowArgIsExplicit = (workflowRef: string | undefined) => (
    !selectedWorkflowRef() || isKnownWorkflowReference(workflowRef)
  )
  return {
    firstWorkflowArgIsExplicit,
    isKnownWorkflowReference,
    selectedWorkflowRef,
    workflowRefOrSelected,
  }
}
