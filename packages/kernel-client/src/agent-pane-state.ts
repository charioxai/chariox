import {
  externalProviderObservedEntryBelongsToImport,
  type ExternalProviderImportMatchFields,
} from "./external-provider-observation.js"
import {
  prependTranscriptEntriesWithoutDuplicateRenderableLineage,
  stripTranscriptDisplayOnlyEntries,
  transcriptEntriesContainRenderableLineage,
  transcriptEntriesShareRenderableLineage,
} from "./transcript-entry-lineage.js"

export type AgentPaneExternalProviderImport = ExternalProviderImportMatchFields

export type AgentPaneSession<TAgent extends { id: string }> = {
  readonly agents: readonly TAgent[]
  readonly focused_agent_id: string | null
}

export type AgentPaneLineageEntry = {
  readonly role: string
  readonly text: string
  readonly turnId?: number
  readonly source?: string | null
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
  readonly historyBlobId?: string
  readonly historyBlobAgentId?: string
  readonly historyBlobSourceId?: string
  readonly historyBlobSourceAgentId?: string
}

export type AgentPaneHistoryBlobEntry = AgentPaneLineageEntry & {
  readonly historyBlobLoaded?: boolean
}

export function selectCurrentAgentPaneEntries<TEntry extends object>(options: {
  readonly agentId: string
  readonly visibleAgentId: string | null
  readonly visibleEntries: readonly TEntry[]
  readonly paneEntriesByAgent: Record<string, readonly TEntry[] | undefined>
}): TEntry[] {
  if (options.agentId === options.visibleAgentId) {
    return options.visibleEntries.map((entry) => ({ ...entry }))
  }
  return (options.paneEntriesByAgent[options.agentId] ?? []).map((entry) => ({ ...entry }))
}

export function shouldRefreshAgentPanesForSessionChange<TAgent extends { id: string }>(options: {
  readonly previousAgents: readonly TAgent[]
  readonly nextAgents: readonly TAgent[]
  readonly splitAgentResponseMode: boolean
  readonly currentFocusedAgentId: string | null
  readonly nextFocusedAgentId: string | null
}): boolean {
  const previousAgentSignature = options.previousAgents.map((agent) => agent.id).join(",")
  const nextAgentSignature = options.nextAgents.map((agent) => agent.id).join(",")
  if (nextAgentSignature !== previousAgentSignature) {
    return true
  }
  if (options.splitAgentResponseMode) {
    return false
  }
  return options.nextFocusedAgentId !== options.currentFocusedAgentId
}

export function countRenderablePaneEntries<TEntry extends { role: string }>(
  entries: readonly TEntry[],
): number {
  return stripTranscriptDisplayOnlyEntries(entries).length
}

export function totalPaneTextLength<TEntry extends { text: string }>(
  entries: readonly TEntry[],
): number {
  return entries.reduce((sum, entry) => sum + entry.text.length, 0)
}

export function shouldPreferCurrentPaneEntries<TEntry extends AgentPaneLineageEntry>(
  currentEntries: readonly TEntry[],
  refreshedEntries: readonly TEntry[],
): boolean {
  if (currentEntries.length === 0) {
    return false
  }
  if (!entriesShareLineage(currentEntries, refreshedEntries)) {
    return false
  }
  if (!refreshedEntriesAreContainedInCurrent(currentEntries, refreshedEntries)) {
    return false
  }

  const currentRenderableCount = countRenderablePaneEntries(currentEntries)
  const refreshedRenderableCount = countRenderablePaneEntries(refreshedEntries)
  if (currentRenderableCount > refreshedRenderableCount) {
    return true
  }

  if (currentRenderableCount < refreshedRenderableCount) {
    return false
  }

  return totalPaneTextLength(currentEntries) > totalPaneTextLength(refreshedEntries)
}

export function refreshedEntriesAreContainedInCurrent<TEntry extends AgentPaneLineageEntry>(
  currentEntries: readonly TEntry[],
  refreshedEntries: readonly TEntry[],
): boolean {
  return transcriptEntriesContainRenderableLineage(currentEntries, refreshedEntries)
}

export function entriesShareLineage<TEntry extends AgentPaneLineageEntry>(
  currentEntries: readonly TEntry[],
  refreshedEntries: readonly TEntry[],
): boolean {
  return transcriptEntriesShareRenderableLineage(currentEntries, refreshedEntries)
}

export function prependHistoryEntriesWithoutDuplicates<TEntry extends AgentPaneLineageEntry>(
  olderEntries: readonly TEntry[],
  currentEntries: readonly TEntry[],
): TEntry[] {
  return prependTranscriptEntriesWithoutDuplicateRenderableLineage(olderEntries, currentEntries)
}

export function trimAgentPaneEntries<TEntry extends { text: string; mergeKey?: string }>(options: {
  readonly entries: TEntry[]
  readonly maxEntries: number
  readonly maxChars: number
  readonly onTrimmedMergeKey?: (mergeKey: string) => void
}): TEntry[] {
  const { entries, maxEntries, maxChars, onTrimmedMergeKey } = options
  let totalChars = entries.reduce((sum, entry) => sum + entry.text.length, 0)
  let removeCount = 0

  while (
    entries.length - removeCount > maxEntries
    || (totalChars > maxChars && removeCount < entries.length - 1)
  ) {
    totalChars -= entries[removeCount]?.text.length ?? 0
    removeCount += 1
  }

  if (removeCount === 0) {
    return entries
  }

  for (const entry of entries.slice(0, removeCount)) {
    if (entry.mergeKey) {
      onTrimmedMergeKey?.(entry.mergeKey)
    }
  }

  return entries.slice(removeCount)
}

export function preserveLoadedHistoryBlobs<TEntry extends AgentPaneHistoryBlobEntry>(options: {
  readonly refreshedEntries: TEntry[]
  readonly currentEntries: readonly TEntry[]
  readonly expandedTurnIds: readonly number[]
  readonly applyExpandedTurns: (entries: TEntry[], expandedTurnIds: readonly number[]) => TEntry[]
  readonly reindexEntries: (entries: TEntry[], startingId: number) => TEntry[]
}): TEntry[] {
  const loadedByBlob = new Map<string, TEntry[]>()
  for (const entry of options.currentEntries) {
    if (!entry.historyBlobLoaded || !entry.historyBlobSourceId) {
      continue
    }
    const key = historyBlobSourceKey(entry.historyBlobSourceAgentId, entry.historyBlobSourceId, entry.turnId)
    const entries = loadedByBlob.get(key) ?? []
    entries.push({ ...entry })
    loadedByBlob.set(key, entries)
  }
  if (loadedByBlob.size === 0) {
    return options.refreshedEntries
  }

  let replaced = false
  const nextEntries = options.refreshedEntries.flatMap((entry) => {
    if (!entry.historyBlobId) {
      return [entry]
    }
    const loadedEntries = loadedByBlob.get(historyBlobSourceKey(entry.historyBlobAgentId, entry.historyBlobId, entry.turnId))
    if (!loadedEntries?.length) {
      return [entry]
    }
    replaced = true
    return loadedEntries.map((loadedEntry) => ({ ...loadedEntry }))
  })
  if (!replaced) {
    return options.refreshedEntries
  }
  return options.reindexEntries(
    options.applyExpandedTurns(nextEntries, options.expandedTurnIds),
    0,
  )
}

export function entryBelongsToAgent(
  agent: { readonly external_provider_import?: AgentPaneExternalProviderImport | null },
  entry: {
    readonly source?: string | null
    readonly externalProvider?: string | null
    readonly externalProviderSessionId?: string | null
  },
): boolean {
  return externalProviderObservedEntryBelongsToImport(agent.external_provider_import, entry)
}

export function historyBlobSourceKey(
  agentId: string | null | undefined,
  blobId: string,
  turnId: number | undefined,
): string {
  return `${agentId ?? ""}:${blobId}:${turnId ?? ""}`
}

export function focusedAgentIdForAgentPaneSession<TAgent extends { id: string }>(
  session: AgentPaneSession<TAgent>,
): string | null {
  const focusedAgentId = session.focused_agent_id
  if (focusedAgentId && session.agents.some((agent) => agent.id === focusedAgentId)) {
    return focusedAgentId
  }
  return session.agents[0]?.id ?? null
}
