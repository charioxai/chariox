import type { QueuedWorkflowLaunch, WorkflowRun } from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"
import {
  formatWorkflowInvocationPrompt,
  validateWorkflowPromptSubmit,
  type WorkflowPromptState,
} from "./workflow-prompt-state.js"

type WorkflowEndpointInvokePayload =
  | { workflow_run: WorkflowRun }
  | { queued_launch: QueuedWorkflowLaunch }

export type WorkflowPromptSubmitControllerDeps = {
  getWorkflowPromptState: () => WorkflowPromptState
  getPendingAttachmentCount: () => number
  beginSubmittedPromptUi: (rawPrompt: string) => SubmittedPromptUiSnapshot
  restoreFailedPromptUi: (snapshot: SubmittedPromptUiSnapshot | null | undefined) => boolean
  invokeWorkflowEndpoint: (
    workflowId: string,
    endpointId: string,
    prompt: string,
  ) => Promise<WorkflowEndpointInvokePayload>
  getSessionId: () => string
  recordPromptAreaHistoryEntry: (sessionId: string, rawPrompt: string) => void
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

      let submissionUi: SubmittedPromptUiSnapshot | null = null
      try {
        submissionUi = deps.beginSubmittedPromptUi(rawPrompt)
        const payload = await deps.invokeWorkflowEndpoint(
          submitDecision.workflowId,
          submitDecision.endpointId,
          formatWorkflowInvocationPrompt(rawPrompt),
        )
        if ("workflow_run" in payload) {
          deps.flashFooter(
            `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
            "info",
          )
        } else {
          deps.flashFooter(`queued workflow launch ${payload.queued_launch.id}`, "info")
        }
        deps.recordPromptAreaHistoryEntry(deps.getSessionId(), rawPrompt)
      } catch (error) {
        deps.restoreFailedPromptUi(submissionUi)
        deps.flashFooter(formatError(error), "error")
      }
    },
  }
}
