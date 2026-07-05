import { navigatePromptHistory } from "@arroba/kernel-client/prompt-history"

type PromptHistoryNavigationControllerOptions = {
  getPromptText: () => string
  getEntries: () => readonly string[]
  getNavigationIndex: () => number | null
  getNavigationDraft: () => string | null
  setNavigationIndex: (index: number | null) => void
  setNavigationDraft: (draft: string | null) => void
  setPromptText: (text: string) => void
  getSessionId: () => string | null
  schedulePromptDraftPersist: (sessionId: string, draft: string) => void
  retainPromptFocus: () => void
}

export type PromptHistoryNavigationController = {
  navigate(direction: "previous" | "next"): boolean
}

export function createPromptHistoryNavigationController(
  options: PromptHistoryNavigationControllerOptions,
): PromptHistoryNavigationController {
  return {
    navigate(direction) {
      const currentText = options.getPromptText()
      const next = navigatePromptHistory({
        entries: options.getEntries(),
        currentText,
        navigationIndex: options.getNavigationIndex(),
        navigationDraft: options.getNavigationDraft(),
        direction,
      })
      if (next.navigationIndex === options.getNavigationIndex() && next.text === currentText) {
        return false
      }

      options.setNavigationIndex(next.navigationIndex)
      options.setNavigationDraft(next.navigationDraft)
      options.setPromptText(next.text)
      const sessionId = options.getSessionId()
      if (sessionId) {
        options.schedulePromptDraftPersist(sessionId, next.navigationDraft ?? next.text)
      }
      options.retainPromptFocus()
      return true
    },
  }
}
