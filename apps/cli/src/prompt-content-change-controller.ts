import {
  derivePromptContentChangeDecision,
  type PromptContentChangeDecision,
} from "./prompt-content-change-policy.js"

type DroppedPromptAttachment = Extract<PromptContentChangeDecision, { kind: "drop" }>["files"][number]

type PromptContentChangeControllerOptions = {
  getPromptText: () => string | null
  isAttached: () => boolean
  getPreviousSnapshot: () => string
  isProgrammaticMutation: () => boolean
  isPromptHistoryActive: () => boolean
  getSessionId: () => string | null | undefined
  getCwd: () => string
  setPromptTextSnapshot: (text: string) => void
  resetPromptHistory: (draft: string) => void
  syncPendingAttachmentsFromText: (text: string) => void
  setPromptText: (text: string) => void
  syncCommandCenter: (text: string) => void
  schedulePromptDraftPersist: (sessionId: string, draft: string) => void
  attachPromptFiles: (files: DroppedPromptAttachment[], insertAt: number) => Promise<void>
  onDropFailed: (error: unknown, files: DroppedPromptAttachment[]) => void
}

export type PromptContentChangeController = {
  handleChange(): boolean
  isDropPending(): boolean
}

export function createPromptContentChangeController(
  options: PromptContentChangeControllerOptions,
): PromptContentChangeController {
  let dropPending = false

  const persistDraft = (draft: { sessionId: string; text: string } | null) => {
    if (draft) {
      options.schedulePromptDraftPersist(draft.sessionId, draft.text)
    }
  }

  return {
    handleChange() {
      const value = options.getPromptText()
      if (value === null) {
        return false
      }

      const decision = derivePromptContentChangeDecision({
        attached: options.isAttached(),
        currentText: value,
        previousSnapshot: options.getPreviousSnapshot(),
        programmaticMutation: options.isProgrammaticMutation(),
        dropPending,
        promptHistoryActive: options.isPromptHistoryActive(),
        sessionId: options.getSessionId(),
        cwd: options.getCwd(),
      })

      if (decision.kind === "detached" || decision.kind === "programmatic") {
        options.setPromptTextSnapshot(decision.nextSnapshot)
        options.syncCommandCenter(decision.commandCenterText)
        return true
      }

      if (decision.resetPromptHistory) {
        options.resetPromptHistory(value)
      }

      if (decision.kind === "text") {
        options.syncPendingAttachmentsFromText(decision.syncAttachmentText)
        options.setPromptTextSnapshot(decision.nextSnapshot)
        options.syncCommandCenter(decision.commandCenterText)
        persistDraft(decision.persistDraft)
        return true
      }

      options.setPromptText(decision.nextPromptText)
      options.syncCommandCenter(decision.commandCenterText)
      persistDraft(decision.persistDraft)
      dropPending = true
      void options.attachPromptFiles(decision.files, decision.insertAt)
        .catch((error) => {
          options.onDropFailed(error, decision.files)
        })
        .finally(() => {
          dropPending = false
        })
      return true
    },
    isDropPending() {
      return dropPending
    },
  }
}
