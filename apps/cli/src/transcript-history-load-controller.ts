import type {
  SessionHistoryCursor,
  SessionHistoryPage,
  TranscriptEntry,
} from "./cli-types.js"
import { hydrateTranscriptEntries } from "./transcript-history.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

type TranscriptHistoryLoadControllerOptions = {
  isAttached: () => boolean
  isLoading: () => boolean
  getCursor: () => SessionHistoryCursor | null
  getSessionId: () => string
  getVisibleAgentId: () => string | null
  getEntryCounter: () => number
  setLoading: (loading: boolean) => void
  setNextCursor: (cursor: SessionHistoryCursor | null) => void
  loadPage: (sessionId: string, cursor: SessionHistoryCursor | null, agentId: string | null) => Promise<SessionHistoryPage>
  prependEntries: (entries: TranscriptEntry[]) => Promise<void>
  flashError: (message: string) => void
  logWarning: (message: string, fields: Record<string, unknown>) => void
  formatError: (error: unknown) => string
}

export type TranscriptHistoryLoadController = {
  bumpGeneration(): void
  loadOlderPage(): Promise<boolean>
}

export function createTranscriptHistoryLoadController(
  options: TranscriptHistoryLoadControllerOptions,
): TranscriptHistoryLoadController {
  let generation = 0

  return {
    bumpGeneration() {
      generation += 1
    },
    async loadOlderPage() {
      const cursor = options.getCursor()
      if (!options.isAttached() || options.isLoading() || cursor === null) {
        return false
      }

      options.setLoading(true)
      const loadGeneration = generation
      const sessionId = options.getSessionId()
      const agentId = options.getVisibleAgentId()
      try {
        let historyPage = await options.loadPage(sessionId, cursor, agentId)
        let hydratedEntries = hydrateTranscriptEntries(historyPage.entries)
        while (hydratedEntries.length > 0 && hydratedEntries[0]?.role !== "user" && historyPage.next_cursor !== null) {
          historyPage = await options.loadPage(sessionId, historyPage.next_cursor, agentId)
          hydratedEntries = [...hydrateTranscriptEntries(historyPage.entries), ...hydratedEntries]
        }
        if (loadGeneration !== generation || !options.isAttached() || options.getSessionId() !== sessionId) {
          return false
        }
        const nextEntries = reindexTranscriptEntries(hydratedEntries, options.getEntryCounter())
        await options.prependEntries(nextEntries)
        options.setNextCursor(historyPage.next_cursor)
        return true
      } catch (error) {
        options.logWarning("older history load failed", {
          error: options.formatError(error),
        })
        options.flashError("failed to load older history")
        return false
      } finally {
        options.setLoading(false)
      }
    },
  }
}
