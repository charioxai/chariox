import type {
  BootstrapDeferredState,
  TranscriptEntry,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

export type DeferredBootstrapControllerDeps = {
  getDeferred: () => BootstrapDeferredState | undefined
  currentAttachmentSessionId: () => string | null
  currentTranscriptEntryCount: () => number
  entryCounter: () => number
  setProviderCatalog: (catalog: ProviderCatalog) => void
  setProviderCommandCatalogs: (catalogs: ProviderCommandCatalogs) => void
  updateSessionChrome: () => void
  setPromptHistoryEntries: (entries: string[]) => void
  resetPromptHistoryNavigation: () => void
  setNextHistoryCursor: (cursor: null) => void
  setAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  setAgentPanePreview: (agentId: string, preview: string) => void
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string | null) => void
  prependTranscriptEntries: (entries: TranscriptEntry[]) => Promise<void>
  logWarning: (message: string, fields: Record<string, unknown>) => void
  formatError: (error: unknown) => string
}

export function createDeferredBootstrapController(deps: DeferredBootstrapControllerDeps) {
  const warn = (message: string, error: unknown) => {
    deps.logWarning(message, {
      error: deps.formatError(error),
    })
  }

  const applyAttachedHistory = async (
    history: Awaited<NonNullable<BootstrapDeferredState["attachedHistory"]>>,
  ) => {
    if (deps.currentAttachmentSessionId() !== history.sessionId) {
      return
    }
    deps.setPromptHistoryEntries(history.promptHistoryEntries)
    deps.resetPromptHistoryNavigation()
    if (!history.visibleAgentId) {
      deps.setNextHistoryCursor(history.nextHistoryCursor)
      return
    }

    for (const [agentId, entries] of Object.entries(history.agentEntries)) {
      const preparedAgentEntries = cloneTranscriptEntries(entries)
      deps.setAgentPaneEntries(agentId, preparedAgentEntries)
      deps.setAgentPanePreview(agentId, formatTranscriptPreview(preparedAgentEntries))
    }

    const visibleAgentId = history.visibleAgentId
    const preparedEntries = cloneTranscriptEntries(history.historyEntries)
    if (preparedEntries.length === 0) {
      deps.setNextHistoryCursor(history.nextHistoryCursor)
      return
    }

    if (deps.currentTranscriptEntryCount() === 0) {
      deps.replaceTranscriptEntries(preparedEntries, visibleAgentId)
    } else {
      await deps.prependTranscriptEntries(reindexTranscriptEntries(preparedEntries, deps.entryCounter()))
    }
    deps.setNextHistoryCursor(history.nextHistoryCursor)
  }

  const apply = () => {
    const deferred = deps.getDeferred()
    if (!deferred) {
      return
    }

    void deferred.providerCatalog?.then((catalog) => {
      deps.setProviderCatalog(catalog)
      deps.updateSessionChrome()
    }).catch((error) => {
      warn("failed to hydrate provider catalog after bootstrap", error)
    })

    void deferred.providerCommandCatalogs?.then((catalogs) => {
      deps.setProviderCommandCatalogs(catalogs)
    }).catch((error) => {
      warn("failed to hydrate provider command catalog after bootstrap", error)
    })

    void deferred.attachedHistory?.then(applyAttachedHistory).catch((error) => {
      warn("failed to hydrate attached history after bootstrap", error)
    })
  }

  return {
    apply,
  }
}

function cloneTranscriptEntries(entries: TranscriptEntry[]) {
  return entries.map((entry) => ({ ...entry }))
}
