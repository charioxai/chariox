import type { FooterFlash } from "./footer-flash-controller.js"
import {
  validateWorkflowPromptSubmit,
  type WorkflowPromptState,
} from "@chariox/kernel-client/workflow-prompt-state"

export type WorkflowPromptSubmitControllerDeps = {
  getWorkflowPromptState: () => WorkflowPromptState
  getPendingAttachmentCount: () => number
  submitAgentPrompt: (rawPrompt: string, targetAgentId: string) => Promise<void>
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  formatError?: (error: unknown) => string
}

export type WorkflowPromptSubmitController = {
  submit(rawPrompt: string): Promise<void>
}

export function createWorkflowPromptSubmitController(
  deps: WorkflowPromptSubmitControllerDeps,
): WorkflowPromptSubmitController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  return {
    async submit(rawPrompt) {
      const submitDecision = validateWorkflowPromptSubmit({
        state: deps.getWorkflowPromptState(),
        pendingAttachmentCount: deps.getPendingAttachmentCount(),
      })
      if (!submitDecision.ok) {
        deps.flashFooter(submitDecision.message, submitDecision.tone)
        return
      }

      try {
        await deps.submitAgentPrompt(rawPrompt, submitDecision.targetAgentId)
      } catch (error) {
        deps.flashFooter(formatError(error), "error")
      }
    },
  }
}
