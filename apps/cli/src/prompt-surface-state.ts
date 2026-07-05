import type { WorkflowPromptState } from "./workflow-prompt-state.js"
import {
  derivePromptAreaBackground as sharedDerivePromptAreaBackground,
  derivePromptInputMaxHeight as sharedDerivePromptInputMaxHeight,
  derivePromptPlaceholder as sharedDerivePromptPlaceholder,
} from "@arroba/kernel-client/prompt-surface-state"

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
  return sharedDerivePromptPlaceholder(options)
}

export function derivePromptAreaBackground<Color>(options: {
  attached: boolean
  workflowScreenActive: boolean
  attachedBackground: Color
  detachedBackground: Color
  workflowBackground: Color
}) {
  return sharedDerivePromptAreaBackground(options)
}

export function derivePromptInputMaxHeight(options: {
  attached: boolean
  terminalHeight: number
}) {
  return sharedDerivePromptInputMaxHeight(options)
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
