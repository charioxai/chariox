import {
  mergeSessionPromptState,
  type CharioxPreferences,
} from "./preferences.js"

export type PromptSessionStateUpdate = {
  promptHistory?: readonly string[]
  promptDraft?: string | null
}

export type PromptSessionStatePersistenceControllerDeps = {
  updatePreferences: (updater: (current: CharioxPreferences) => CharioxPreferences) => void
  savePromptState: (sessionId: string, next: PromptSessionStateUpdate) => Promise<void>
}

export function createPromptSessionStatePersistenceController(
  deps: PromptSessionStatePersistenceControllerDeps,
) {
  const persist = async (sessionId: string, next: PromptSessionStateUpdate) => {
    deps.updatePreferences((current) => mergeSessionPromptState(current, sessionId, next))
    await deps.savePromptState(sessionId, next)
  }

  return {
    persist,
  }
}
