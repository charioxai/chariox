import type {
  PromptAttachmentPart,
  RuntimeAttachment,
  RuntimeSession,
} from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import type { PromptSubmissionResult } from "./prompt-runtime-api.js"
import {
  formatPromptSubmissionBody,
  formatPromptSubmissionStatusLine,
  pendingPromptAttachmentsToParts,
} from "./prompt-submission-state.js"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"
import {
  activePromptIdForAgent,
  agentPromptState,
} from "./session-state.js"

export type NormalPromptSubmitControllerDeps = {
  getPendingAttachments: () => readonly PendingPromptAttachment[]
  waitForPendingAgentFocusTransition: () => Promise<void>
  getFocusedAgentId: () => string | null
  hasAgent: (agentId: string) => boolean
  clearActiveToolLabels: () => void
  setProviderActivityLabel: (label: string | null) => void
  setActiveStatusLabel: (label: string | null) => void
  getAttachment: () => RuntimeAttachment | null
  getSessionId: () => string
  clearPromptText: () => void
  shouldInlineLocalFiles: () => boolean
  preparePromptAttachmentsForSubmit: (
    attachments: PromptAttachmentPart[],
    options: { inlineLocalFiles: boolean },
  ) => Promise<PromptAttachmentPart[]>
  beginSubmittedPromptUi: (rawPrompt: string) => SubmittedPromptUiSnapshot
  renderPromptTranscript: (prompt: string) => string
  appendUserPrompt: (text: string, agentId?: string | null) => void
  submitPrompt: (
    attachmentId: string,
    targetAgentId: string | null,
    prompt: string,
    attachments: PromptAttachmentPart[],
  ) => Promise<PromptSubmissionResult>
  applySessionState: (session: RuntimeSession) => void
  setStreamingAgentId: (agentId: string | null) => void
  setWorking: (working: boolean) => void
  updateSessionChrome: () => void
  setStatusLine: (line: string) => void
  recordPromptAreaHistoryEntry: (sessionId: string, rawPrompt: string) => void
  restoreFailedPromptUi: (snapshot: SubmittedPromptUiSnapshot | null | undefined) => boolean
  getSubmittingAgentId: () => string | null
  clearAgentBusy: (agentId: string | null | undefined) => void
  setSubmittingAgentId: (agentId: string | null) => void
  setSubmitting: (submitting: boolean) => void
  setFatalError: (message: string) => void
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  logInfo?: (message: string, fields: Record<string, unknown>) => void
  logError?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type NormalPromptSubmitController = {
  submit(rawPrompt: string, targetAgentIdOverride?: string | null): Promise<void>
}

export function createNormalPromptSubmitController(
  deps: NormalPromptSubmitControllerDeps,
): NormalPromptSubmitController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  return {
    async submit(rawPrompt, targetAgentIdOverride) {
      const prompt = formatPromptSubmissionBody(rawPrompt)
      const rawAttachments = pendingPromptAttachmentsToParts(deps.getPendingAttachments())
      let submissionUi: SubmittedPromptUiSnapshot | null = null
      try {
        await deps.waitForPendingAgentFocusTransition()
        const requestedTargetAgentId = targetAgentIdOverride ?? deps.getFocusedAgentId()
        const targetAgentId = requestedTargetAgentId && deps.hasAgent(requestedTargetAgentId)
          ? requestedTargetAgentId
          : null
        deps.logInfo?.("submitting prompt", {
          chars: prompt.length,
          attachments: rawAttachments.length,
        })
        deps.clearActiveToolLabels()
        deps.setProviderActivityLabel(null)
        deps.setActiveStatusLabel(null)
        const attachment = deps.getAttachment()
        if (!attachment) {
          deps.flashFooter("No session attached.", "error")
          deps.clearPromptText()
          return
        }
        const attachments = await deps.preparePromptAttachmentsForSubmit(rawAttachments, {
          inlineLocalFiles: deps.shouldInlineLocalFiles(),
        })
        submissionUi = deps.beginSubmittedPromptUi(rawPrompt)
        const submission = await deps.submitPrompt(attachment.id, targetAgentId, prompt, attachments)
        const payload = submission.payload
        const submittedTargetAgentId = submission.targetAgentId ?? targetAgentId
        deps.applySessionState(payload.session)
        const outcomeName = submission.outcomeName
        if (outcomeName !== "Queued") {
          deps.appendUserPrompt(deps.renderPromptTranscript(prompt), submittedTargetAgentId)
        }
        deps.setStreamingAgentId(submittedTargetAgentId)
        deps.setWorking(true)
        deps.updateSessionChrome()
        const activePromptId = activePromptIdForAgent(payload.session, submittedTargetAgentId)
        const queuedPromptCount = submittedTargetAgentId
          ? agentPromptState(payload.session, submittedTargetAgentId)?.queued_prompts.length ?? 0
          : payload.session.queued_prompts.length
        deps.logInfo?.("prompt submitted", {
          outcome: outcomeName,
          active_prompt_id: activePromptId,
          queued_prompts: queuedPromptCount,
        })
        deps.setStatusLine(formatPromptSubmissionStatusLine({
          outcomeName,
          activePromptId,
        }))
        deps.updateSessionChrome()
        deps.recordPromptAreaHistoryEntry(deps.getSessionId(), rawPrompt)
      } catch (error) {
        deps.logError?.("prompt submission failed", {
          error: formatError(error),
        })
        deps.restoreFailedPromptUi(submissionUi)
        deps.clearAgentBusy(deps.getSubmittingAgentId())
        deps.setSubmittingAgentId(null)
        deps.setSubmitting(false)
        deps.setWorking(false)
        deps.setFatalError(formatError(error))
        deps.updateSessionChrome()
      }
    },
  }
}
