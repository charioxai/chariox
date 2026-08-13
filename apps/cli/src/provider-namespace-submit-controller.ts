import type {
  RuntimeAttachment,
  RuntimeSession,
} from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import {
  parseProviderNamespaceCommand,
  type ParsedProviderNamespaceCommand,
} from "./provider-command-catalog.js"
import type { BackendProviderId } from "./provider-catalog.js"
import {
  validateProviderNamespaceSubmit,
} from "@chariox/kernel-client/provider-namespace-submit-policy"
import {
  promptSubmissionFailureTransition,
  promptSubmissionSuccessTransition,
  resolvePromptSubmissionTargetAgentId,
} from "@chariox/kernel-client/prompt-submission"
import {
  promptSubmissionTranscriptMetadata,
  type PromptSubmissionResult,
} from "./prompt-runtime-api.js"
import type { TranscriptPromptMetadata } from "@chariox/kernel-client/transcript-entry-state"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"

export type ProviderNamespaceSubmitControllerDeps = {
  getFocusedProvider: () => BackendProviderId | null
  workflowScreenShowing: () => boolean
  getPendingAttachmentCount: () => number
  waitForPendingAgentFocusTransition: () => Promise<void>
  getFocusedAgentId: () => string | null
  getSession: () => RuntimeSession
  hasAgent: (agentId: string) => boolean
  clearActiveToolLabels: () => void
  setProviderActivityLabel: (label: string | null) => void
  setActiveStatusLabel: (label: string | null) => void
  getAttachment: () => RuntimeAttachment | null
  getSessionId: () => string
  clearPromptText: () => void
  beginSubmittedPromptUi: (rawPrompt: string) => SubmittedPromptUiSnapshot
  renderPromptTranscript: (prompt: string) => string
  appendUserPrompt: (text: string, agentId?: string | null, metadata?: TranscriptPromptMetadata) => void
  submitProviderNamespacePrompt: (
    attachmentId: string,
    targetAgentId: string | null,
    forwardedPrompt: string,
  ) => Promise<PromptSubmissionResult>
  applySessionState: (session: RuntimeSession) => void
  setStreamingAgentId: (agentId: string | null) => void
  setWorking: (working: boolean) => void
  updateSessionChrome: () => void
  recordPromptAreaHistoryEntry: (sessionId: string, rawPrompt: string) => void
  clearCommandCenter: () => void
  restoreFailedPromptUi: (snapshot: SubmittedPromptUiSnapshot | null | undefined) => boolean
  getSubmittingAgentId: () => string | null
  clearAgentBusy: (agentId: string | null | undefined) => void
  setSubmittingAgentId: (agentId: string | null) => void
  setSubmitting: (submitting: boolean) => void
  setFatalError: (message: string) => void
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  logError?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type ProviderNamespaceSubmitController = {
  submit(rawPrompt: string): Promise<boolean>
}

export function createProviderNamespaceSubmitController(
  deps: ProviderNamespaceSubmitControllerDeps,
): ProviderNamespaceSubmitController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const validate = (command: ParsedProviderNamespaceCommand) => validateProviderNamespaceSubmit({
    command,
    focusedProvider: deps.getFocusedProvider(),
    workflowScreenShowing: deps.workflowScreenShowing(),
    pendingAttachmentCount: deps.getPendingAttachmentCount(),
  })

  return {
    async submit(rawPrompt) {
      const command = parseProviderNamespaceCommand(rawPrompt, deps.getFocusedProvider())
      if (!command) {
        return false
      }

      const submitDecision = validate(command)
      if (!submitDecision.ok) {
        deps.flashFooter(submitDecision.message, "error")
        return true
      }

      let submissionUi: SubmittedPromptUiSnapshot | null = null
      try {
        await deps.waitForPendingAgentFocusTransition()
        const focusedAgentId = deps.getFocusedAgentId()
        const targetAgentId = resolvePromptSubmissionTargetAgentId({
          requestedTargetAgentId: focusedAgentId,
          hasAgent: deps.hasAgent,
        })
        deps.clearActiveToolLabels()
        deps.setProviderActivityLabel(null)
        deps.setActiveStatusLabel(null)
        const attachment = deps.getAttachment()
        if (!attachment) {
          deps.flashFooter("No session attached.", "error")
          deps.clearPromptText()
          return true
        }
        submissionUi = deps.beginSubmittedPromptUi(rawPrompt)
        const submission = await deps.submitProviderNamespacePrompt(
          attachment.id,
          targetAgentId,
          `${submitDecision.forwardedCommand}\n`,
        )
        const submittedTargetAgentId = submission.targetAgentId ?? targetAgentId
        deps.applySessionState(submission.payload.session)
        const transition = promptSubmissionSuccessTransition({
          session: submission.payload.session,
          outcomeName: submission.outcomeName,
          submittedTargetAgentId,
        })
        if (transition.shouldAppendUserPrompt) {
          deps.appendUserPrompt(
            deps.renderPromptTranscript(command.raw),
            submittedTargetAgentId,
            promptSubmissionTranscriptMetadata(submission.payload, submittedTargetAgentId),
          )
        }
        deps.setStreamingAgentId(transition.streamingAgentId)
        deps.setWorking(transition.working)
        deps.updateSessionChrome()
        deps.recordPromptAreaHistoryEntry(deps.getSessionId(), rawPrompt)
        deps.clearCommandCenter()
      } catch (error) {
        deps.logError?.("provider namespace command failed", {
          command: command.raw,
          error: formatError(error),
        })
        deps.restoreFailedPromptUi(submissionUi)
        const transition = promptSubmissionFailureTransition({
          session: deps.getSession(),
          submittingAgentId: deps.getSubmittingAgentId(),
        })
        deps.clearAgentBusy(transition.clearBusyAgentId)
        deps.setSubmittingAgentId(transition.submittingAgentId)
        deps.setSubmitting(transition.submitting)
        deps.setStreamingAgentId(transition.streamingAgentId)
        deps.setWorking(transition.working)
        deps.setFatalError(formatError(error))
        deps.updateSessionChrome()
      }
      return true
    },
  }
}
