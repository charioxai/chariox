export type PromptSessionHistoryControllerDeps = {
  currentSessionId: () => string | null
  navigationDraft: () => string | null
  currentPromptText: () => string
  scheduleHistoryRefresh: (sessionId: string) => void
}

export function createPromptSessionHistoryController(
  deps: PromptSessionHistoryControllerDeps,
) {
  return {
    scheduleSharedRefresh() {
      const sessionId = deps.currentSessionId()
      if (!sessionId) {
        return false
      }
      deps.scheduleHistoryRefresh(sessionId)
      return true
    },
    persistableDraft() {
      const draft = deps.navigationDraft()
      if (draft !== null) {
        return draft
      }
      return deps.currentPromptText()
    },
  }
}
