import {
  formatWorkflowPromptPlaceholder,
  type WorkflowPromptAgentLike,
  type WorkflowPromptRunLike,
  type WorkflowPromptState,
  type WorkflowPromptWorkflowLike,
} from "./workflow-prompt-state.js"

export function derivePromptPlaceholder(options: {
  attached: boolean
  workflowScreenActive: boolean
  workflowPromptState: WorkflowPromptState<WorkflowPromptWorkflowLike, WorkflowPromptRunLike, WorkflowPromptAgentLike>
  attachedPlaceholder: string
  detachedPlaceholder: string
}): string {
  if (!options.attached) {
    return options.detachedPlaceholder
  }
  return formatWorkflowPromptPlaceholder({
    workflowScreenActive: options.workflowScreenActive,
    state: options.workflowPromptState,
    attachedPlaceholder: options.attachedPlaceholder,
    detachedPlaceholder: options.detachedPlaceholder,
  })
}

export function derivePromptAreaBackground<Color>(options: {
  attached: boolean
  workflowScreenActive: boolean
  attachedBackground: Color
  detachedBackground: Color
  workflowBackground: Color
}): Color {
  if (!options.attached) {
    return options.detachedBackground
  }
  return options.workflowScreenActive
    ? options.workflowBackground
    : options.attachedBackground
}

export function derivePromptInputMaxHeight(options: {
  attached: boolean
  terminalHeight: number
}): number {
  return options.attached
    ? Math.max(6, options.terminalHeight - 11)
    : 6
}
