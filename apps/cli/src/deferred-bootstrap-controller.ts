import { clientInventoryEpoch } from "./client-inventory-cache.js"
import type {
  AttachLaunch,
  BootstrapDeferredState,
  RuntimeProviderRun,
  SessionHistoryCursorState,
  TranscriptEntry,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import type { TerminalCommandCatalog } from "@chariox/kernel-client/kernel-types"
import { formatTranscriptPreview } from "@chariox/kernel-client/session-history-preview"
import { reindexTranscriptEntries } from "@chariox/kernel-client/transcript-entry-state"

export type DeferredBootstrapControllerDeps = {
  applyWaitingRoomDefaults?: (defaults: { provider?: string; model?: string; effort?: string }) => void
  getDeferred: () => BootstrapDeferredState | undefined
  currentAttachmentSessionId: () => string | null
  currentAttachmentId: () => string | null
  currentPromptHistoryEntries?: () => readonly string[]
  currentPromptHistoryIndex?: () => number | null
  currentPromptHistoryDraft?: () => string | null
  focusedAgentId?: () => string | null
  setProviderRun?: (run: RuntimeProviderRun) => void
  providerLaunchFailed?: (message: string) => void
  currentTranscriptEntryCount: () => number
  entryCounter: () => number
  setProviderCatalog: (catalog: ProviderCatalog) => void
  setProviderCommandCatalogs: (catalogs: ProviderCommandCatalogs) => void
  setTerminalCommandCatalog: (catalog: TerminalCommandCatalog) => void
  updateSessionChrome: () => void
  applyAttachLaunch: (launch: AttachLaunch) => void
  reportAttachLaunchFailure: (message: string) => void
  setPromptHistoryEntries: (entries: string[]) => void
  resetPromptHistoryNavigation: () => void
  setNextHistoryCursor: (cursor: SessionHistoryCursorState) => void
  setAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  setSessionHydratingState: (next: boolean) => void
  setAgentPanePreview: (agentId: string, preview: string) => void
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string | null) => void
  prependTranscriptEntries: (entries: TranscriptEntry[]) => Promise<void>
  logWarning: (message: string, fields: Record<string, unknown>) => void
  formatError: (error: unknown) => string
}

export function createDeferredBootstrapController(deps: DeferredBootstrapControllerDeps) {
  let applied = false
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
    if (history.includesPromptHistory !== false) {
      deps.setPromptHistoryEntries(history.promptHistoryEntries)
      deps.resetPromptHistoryNavigation()
    }
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
    if (applied) {
      return
    }
    applied = true
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

    void deferred.waitingRoomDefaults?.then(defaults => deps.applyWaitingRoomDefaults?.(defaults)).catch(error => {
      warn("failed to hydrate waiting-room defaults", error)
    })

    void deferred.providerCommandCatalogs?.then((catalogs) => {
      deps.setProviderCommandCatalogs(catalogs)
    }).catch((error) => {
      warn("failed to hydrate provider command catalog after bootstrap", error)
    })

    void deferred.terminalCommandCatalog?.then((catalog) => {
      deps.setTerminalCommandCatalog(catalog)
    }).catch((error) => {
      warn("failed to hydrate terminal command catalog after bootstrap", error)
    })

    const launchAttachmentId = deps.currentAttachmentId()
    void deferred.attachLaunch?.then(launch => {
      if (deps.currentAttachmentId() === launchAttachmentId && deps.currentAttachmentSessionId() === launch.sessionId) deps.applyAttachLaunch(launch)
    }).catch(error => {
      warn("attach-time provider launch failed", error)
      if (deps.currentAttachmentId() === launchAttachmentId) deps.reportAttachLaunchFailure(deps.formatError(error))
    })

    // Only the startup attachment's hydration ends here; a newer attach owns its own.
    const startupAttachmentId = deps.currentAttachmentId()
    const startupPromptHistory = deps.currentPromptHistoryEntries?.()
    const startupPromptIndex = deps.currentPromptHistoryIndex?.()
    const startupPromptDraft = deps.currentPromptHistoryDraft?.()
    void deferred.promptInputHistory?.then(history => {
      if (deps.currentAttachmentId() !== startupAttachmentId
        || deps.currentAttachmentSessionId() !== history.sessionId
        || deps.currentPromptHistoryEntries?.() !== startupPromptHistory
        || deps.currentPromptHistoryIndex?.() !== startupPromptIndex
        || deps.currentPromptHistoryDraft?.() !== startupPromptDraft) return
      deps.setPromptHistoryEntries(history.entries)
      deps.resetPromptHistoryNavigation()
    }).catch(error => {
      warn("failed to hydrate prompt input history after bootstrap", error)
    })
    void deferred.attachedHistory?.then(applyAttachedHistory).catch((error) => {
      warn("failed to hydrate attached history after bootstrap", error)
    }).finally(() => {
      if (deps.currentAttachmentId() === startupAttachmentId) {
        deps.setSessionHydratingState(false)
      }
    })

    const launch = deferred.providerLaunch
    if (launch && deps.currentAttachmentSessionId() === launch.sessionId) {
      void launch.start().then(({ providerRun }) => {
        if (deps.currentAttachmentSessionId() !== launch.sessionId) return
        if (providerRun && (!deps.focusedAgentId || !providerRun.agent_instance_id
          || providerRun.agent_instance_id === deps.focusedAgentId())) {
          deps.setProviderRun?.(providerRun)
        }
        deps.updateSessionChrome()
      }).catch(error => {
        warn("provider launch after terminal mount failed", error)
        if (deps.currentAttachmentSessionId() === launch.sessionId) {
          deps.providerLaunchFailed?.(deps.formatError(error))
        }
      })
    }
  }

  let promptFlight: Promise<void> | undefined, promptAttachment: string | null | undefined
  const hydratePromptInputHistory = () => {
    const attachment = deps.currentAttachmentId(), sessionId = deps.currentAttachmentSessionId()
    if (promptAttachment !== attachment) { promptFlight = undefined; promptAttachment = attachment }
    if (!sessionId) return Promise.resolve()
    return promptFlight ??= (async () => {
      const history = await deps.getDeferred()?.loadPromptInputHistory?.(sessionId)
      if (!history || attachment !== deps.currentAttachmentId() || history.sessionId !== deps.currentAttachmentSessionId()) return
      if (deps.currentPromptHistoryIndex?.() == null) deps.setPromptHistoryEntries(history.entries)
    })().catch(error => { promptFlight = undefined; warn("failed to load prompt navigation", error) })
  }
  let composerFlight: Promise<void> | undefined
  let composerEpoch = clientInventoryEpoch(), composerLoadedAt = 0
  let composerAttachment: string | null | undefined, composerScope: string | undefined
  const hydrateComposer = () => {
    const load = deps.getDeferred()?.loadComposer
    if (!load) return Promise.resolve()
    const selectedScope = deps.getDeferred()?.composerScopeKey?.()
    if (composerAttachment !== deps.currentAttachmentId() || composerScope !== selectedScope
      || composerEpoch !== clientInventoryEpoch() || Date.now() - composerLoadedAt >= 4 * 60 * 60 * 1000) {
      composerAttachment = deps.currentAttachmentId(); composerScope = selectedScope
      composerFlight = undefined; composerEpoch = clientInventoryEpoch(); composerLoadedAt = Date.now()
    }
    if (composerFlight) return composerFlight
    const attachment = deps.currentAttachmentId()
    const applyComposer = (data: import("./cli-types.js").ComposerInventory) => {
      if (attachment !== deps.currentAttachmentId() || selectedScope !== deps.getDeferred()?.composerScopeKey?.()) return
      deps.setProviderCatalog(data.providerCatalog)
      deps.setProviderCommandCatalogs(data.providerCommandCatalogs)
      deps.setTerminalCommandCatalog(data.terminalCommandCatalog)
      deps.updateSessionChrome()
    }
    composerFlight = load().then(data => {
      applyComposer(data)
      void data.reconciliation?.then(applyComposer).catch(error => warn("failed to reconcile composer inventory", error))
    }).catch(error => { composerFlight = undefined; warn("failed to load composer inventory", error) })
    return composerFlight
  }

  return {
    hydrateComposer,
    hydratePromptInputHistory,
    apply,
  }
}

function cloneTranscriptEntries(entries: TranscriptEntry[]) {
  return entries.map((entry) => ({ ...entry }))
}
