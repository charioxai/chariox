import {
  promptHistoryEntryListsEqual,
  pushPromptHistoryEntry,
} from "@arroba/kernel-client/prompt-history"

type PromptInputHistoryEntry = {
  sequence: number
  text: string
}

type PromptSessionStateUpdate = {
  promptHistory?: readonly string[]
  promptDraft?: string | null
}

type PromptInputHistoryControllerOptions = {
  getCurrentSessionId: () => string | null
  getAttachmentId: () => string | null
  getEntries: () => readonly string[]
  setEntries: (entries: readonly string[]) => void
  resetNavigation: () => void
  clearDraftPersistQueue: () => void
  persistPromptState: (sessionId: string, next: PromptSessionStateUpdate) => Promise<void>
  recordPromptInputHistory: (
    sessionId: string,
    attachmentId: string | null,
    kind: "command",
    text: string,
  ) => Promise<PromptInputHistoryEntry>
  onSharedHistoryPersistFailed: (sessionId: string, error: unknown) => void
  onPromptEchoPersistFailed: (sessionId: string, error: unknown) => void
  onPromptStatePersistFailed: (sessionId: string, error: unknown) => void
  onRecordSharedHistoryFailed: (sessionId: string, error: unknown) => void
}

export type PromptInputHistoryController = {
  latestSequence(): number
  setLatestSequence(sequence: number): void
  replaceFromHydration(
    sessionId: string,
    entries: readonly string[],
    latestSequence: number,
  ): Promise<void>
  appendShared(sessionId: string, entries: readonly PromptInputHistoryEntry[]): boolean
  appendEcho(text: string): boolean
  recordPromptAreaEntry(sessionId: string | null, rawPrompt: string): boolean
}

export function createPromptInputHistoryController(
  options: PromptInputHistoryControllerOptions,
): PromptInputHistoryController {
  let latestSequence = 0

  const persistHistory = (
    sessionId: string,
    entries: readonly string[],
    onError: (sessionId: string, error: unknown) => void,
  ) => {
    void options.persistPromptState(sessionId, {
      promptHistory: entries,
    }).catch((error) => {
      onError(sessionId, error)
    })
  }

  const controller: PromptInputHistoryController = {
    latestSequence() {
      return latestSequence
    },
    setLatestSequence(sequence) {
      latestSequence = sequence
    },
    async replaceFromHydration(sessionId, entries, nextLatestSequence) {
      latestSequence = nextLatestSequence
      options.setEntries(entries)
      options.resetNavigation()
      await options.persistPromptState(sessionId, {
        promptHistory: entries,
      })
    },
    appendShared(sessionId, entries) {
      if (options.getCurrentSessionId() !== sessionId || entries.length === 0) {
        return false
      }

      const currentEntries = options.getEntries()
      let nextEntries = currentEntries
      for (const entry of [...entries].sort((left, right) => left.sequence - right.sequence)) {
        latestSequence = Math.max(latestSequence, entry.sequence)
        nextEntries = pushPromptHistoryEntry(nextEntries, entry.text)
      }
      if (promptHistoryEntryListsEqual(nextEntries, currentEntries)) {
        return false
      }

      options.setEntries(nextEntries)
      persistHistory(sessionId, nextEntries, options.onSharedHistoryPersistFailed)
      return true
    },
    appendEcho(text) {
      const sessionId = options.getCurrentSessionId()
      if (!sessionId) {
        return false
      }

      const currentEntries = options.getEntries()
      const nextEntries = pushPromptHistoryEntry(currentEntries, text)
      if (promptHistoryEntryListsEqual(nextEntries, currentEntries)) {
        return false
      }

      options.setEntries(nextEntries)
      persistHistory(sessionId, nextEntries, options.onPromptEchoPersistFailed)
      return true
    },
    recordPromptAreaEntry(sessionId, rawPrompt) {
      if (!sessionId) {
        return false
      }

      const nextEntries = pushPromptHistoryEntry(options.getEntries(), rawPrompt)
      options.setEntries(nextEntries)
      options.resetNavigation()
      options.clearDraftPersistQueue()
      void options.persistPromptState(sessionId, {
        promptHistory: nextEntries,
        promptDraft: "",
      }).catch((error) => {
        options.onPromptStatePersistFailed(sessionId, error)
      })

      if (rawPrompt.trimStart().startsWith("/")) {
        void options.recordPromptInputHistory(
          sessionId,
          options.getAttachmentId(),
          "command",
          rawPrompt.trimEnd(),
        ).then((entry) => {
          controller.appendShared(sessionId, [entry])
        }).catch((error) => {
          options.onRecordSharedHistoryFailed(sessionId, error)
        })
      }
      return true
    },
  }

  return controller
}
