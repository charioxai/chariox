import type { ArrobaPreferences } from "./preferences.js"
import {
  sessionPromptDraftEntry,
  sessionPromptHistoryEntries,
} from "./preferences.js"

export type PromptHistoryRestoreControllerDeps = {
  getPreferences: () => ArrobaPreferences
  setPromptHistoryEntries: (entries: string[]) => void
  resetPromptHistoryNavigation: () => void
  setPromptText: (text: string) => void
}

export function createPromptHistoryRestoreController(
  deps: PromptHistoryRestoreControllerDeps,
) {
  const restore = (sessionId: string | null) => {
    const preferences = deps.getPreferences()
    deps.setPromptHistoryEntries(sessionId ? sessionPromptHistoryEntries(preferences, sessionId) : [])
    deps.resetPromptHistoryNavigation()
    deps.setPromptText(sessionId ? sessionPromptDraftEntry(preferences, sessionId) : "")
  }

  return {
    restore,
  }
}
