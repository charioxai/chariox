import type { PendingPromptAttachment } from "./prompt-attachment-state.js"

export type SubmittedPromptUiSnapshot = {
  rawPrompt: string
  attachments: PendingPromptAttachment[]
  sessionId: string | null
}

type PromptSubmissionUiControllerOptions = {
  getSessionId: () => string | null
  getPendingAttachments: () => readonly PendingPromptAttachment[]
  resetPromptHistoryNavigation: () => void
  clearDraftPersistQueue: () => void
  clearPromptText: () => void
  setPromptText: (text: string) => void
  syncPromptTextSnapshot: () => void
  clearPendingAttachments: () => void
  setPendingAttachments: (attachments: PendingPromptAttachment[]) => void
  refreshAttachmentHighlights: () => void
  syncCommandCenter: (text: string) => void
  retainPromptFocus: () => void
  clearCommandCenter: () => void
  schedulePromptDraftPersist: (sessionId: string, draft: string) => void
  updateSessionChrome: () => void
}

export type PromptSubmissionUiController = {
  begin(rawPrompt: string): SubmittedPromptUiSnapshot
  restore(snapshot: SubmittedPromptUiSnapshot | null | undefined): boolean
}

export function createPromptSubmissionUiController(
  options: PromptSubmissionUiControllerOptions,
): PromptSubmissionUiController {
  const resetPromptHistory = () => {
    options.resetPromptHistoryNavigation()
    options.clearDraftPersistQueue()
  }

  return {
    begin(rawPrompt) {
      const snapshot: SubmittedPromptUiSnapshot = {
        rawPrompt,
        attachments: options.getPendingAttachments().map((file) => ({ ...file })),
        sessionId: options.getSessionId(),
      }

      resetPromptHistory()
      options.clearPromptText()
      options.syncPromptTextSnapshot()
      options.clearPendingAttachments()
      options.syncCommandCenter("")
      options.retainPromptFocus()
      options.clearCommandCenter()
      if (snapshot.sessionId) {
        options.schedulePromptDraftPersist(snapshot.sessionId, "")
      }
      return snapshot
    },
    restore(snapshot) {
      if (!snapshot) {
        return false
      }

      options.resetPromptHistoryNavigation()
      options.setPendingAttachments(snapshot.attachments.map((file) => ({ ...file })))
      options.setPromptText(snapshot.rawPrompt)
      options.syncPromptTextSnapshot()
      options.refreshAttachmentHighlights()
      options.syncCommandCenter(snapshot.rawPrompt)
      options.retainPromptFocus()
      if (snapshot.sessionId) {
        options.schedulePromptDraftPersist(snapshot.sessionId, snapshot.rawPrompt)
      }
      options.updateSessionChrome()
      return true
    },
  }
}
