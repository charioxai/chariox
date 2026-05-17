import {
  formatWorkflowPromptPlaceholder,
  type WorkflowPromptState,
} from "./workflow-prompt-state.js"

export type PromptInputPlaceholderTarget = {
  placeholder: unknown
}

export function derivePromptPlaceholder(options: {
  attached: boolean
  workflowScreenActive: boolean
  workflowPromptState: WorkflowPromptState
  attachedPlaceholder: string
  detachedPlaceholder: string
}) {
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
}) {
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
}) {
  return options.attached
    ? Math.max(6, options.terminalHeight - 11)
    : 6
}

export function createPromptPlaceholderSyncController(deps: {
  getPromptInput: () => PromptInputPlaceholderTarget | null
  getPlaceholder: () => string
}) {
  return {
    sync() {
      const promptInput = deps.getPromptInput()
      if (!promptInput) {
        return
      }
      promptInput.placeholder = deps.getPlaceholder()
    },
  }
}
