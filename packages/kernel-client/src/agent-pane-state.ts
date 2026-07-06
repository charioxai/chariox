import {
  externalProviderObservedEntryBelongsToImport,
  type ExternalProviderImportMatchFields,
} from "./external-provider-observation.js"
import {
  prependTranscriptEntriesWithoutDuplicateRenderableLineage,
  stripTranscriptDisplayOnlyEntries,
  transcriptEntriesContainRenderableLineage,
  transcriptEntriesShareRenderableLineage,
  type TranscriptLineageEntry,
  type TranscriptRoleEntry,
} from "./transcript-entry-lineage.js"
import { transcriptRetentionSlice } from "./transcript-entry-state.js"

export type AgentPaneExternalProviderImport = ExternalProviderImportMatchFields

export type AgentPaneSession<TAgent extends { id: string }> = {
  readonly agents: readonly TAgent[]
  readonly focused_agent_id: string | null
}

export type AgentPaneLineageEntry = TranscriptLineageEntry

export type AgentPaneHistoryBlobEntry = AgentPaneLineageEntry & {
  readonly historyBlobLoaded?: boolean
}

export type AgentPaneRefreshResult<TEntry, TCursor> = {
  readonly paneEntries: Record<string, TEntry[]>
  readonly previews: Record<string, string>
  readonly collapsedTurnIdsByAgent: Record<string, number[]>
  readonly visibleAgentId: string | null
  readonly visibleEntries: TEntry[]
  readonly visibleCursor: TCursor | null
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

export function countRenderablePaneEntries<TEntry extends TranscriptRoleEntry>(
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
  const retention = transcriptRetentionSlice(options.entries, {
    maxEntries: options.maxEntries,
    maxChars: options.maxChars,
  })
  if (!retention.changed) {
    return options.entries
  }

  for (const entry of retention.removed) {
    if (entry.mergeKey) {
      options.onTrimmedMergeKey?.(entry.mergeKey)
    }
  }

  return retention.kept
}

export function preserveLoadedHistoryBlobs<TEntry extends AgentPaneHistoryBlobEntry>(options: {
  readonly refreshedEntries: TEntry[]
  readonly currentEntries: readonly TEntry[]
  readonly collapsedTurnIds: readonly number[]
  readonly applyCollapsedTurns: (entries: TEntry[], collapsedTurnIds: readonly number[]) => TEntry[]
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
    options.applyCollapsedTurns(nextEntries, options.collapsedTurnIds),
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

function historyCursorKey(cursor: unknown): string {
  return JSON.stringify(cursor)
}

export async function refreshAgentPaneState<
  TAgent extends { id: string; external_provider_import?: AgentPaneExternalProviderImport | null },
  THistoryEntry,
  TEntry extends AgentPaneHistoryBlobEntry,
  TCursor,
>(options: {
  readonly session: AgentPaneSession<TAgent>
  readonly hasPromptWorkForAgent: (agent: TAgent) => boolean
  readonly collapsedTurnIdsByAgent: Record<string, readonly number[] | undefined>
  readonly currentPaneEntriesByAgent?: Record<string, readonly TEntry[] | undefined>
  readonly resolveVisibleAgentId: (agents: readonly TAgent[], focusedAgentId: string | null) => string | null
  readonly loadHistoryPage: (
    agentId: string,
    cursor: TCursor | null,
  ) => Promise<{ entries: THistoryEntry[]; nextCursor: TCursor | null }>
  readonly hydrateEntries: (entries: THistoryEntry[]) => TEntry[]
  readonly collapseHistoricalTurns: (entries: TEntry[], keepLatestExpanded: boolean) => TEntry[]
  readonly applyCollapsedTurns: (entries: TEntry[], collapsedTurnIds: readonly number[]) => TEntry[]
  readonly reindexEntries: (entries: TEntry[], startingId: number) => TEntry[]
  readonly formatPreview: (entries: TEntry[]) => string
  readonly preserveCollapsedTurnIds?: boolean
}): Promise<AgentPaneRefreshResult<TEntry, TCursor>> {
  const previews: Record<string, string> = {}
  const paneEntries: Record<string, TEntry[]> = {}
  const collapsedTurnIdsByAgent: Record<string, number[]> = {}
  const visibleAgentId = options.resolveVisibleAgentId(
    options.session.agents,
    focusedAgentIdForAgentPaneSession(options.session),
  )
  let visibleEntries: TEntry[] = []
  let visibleCursor: TCursor | null = null

  for (const agent of options.session.agents) {
    const agentHasPromptWork = options.hasPromptWorkForAgent(agent)
    const currentPaneEntries = (options.currentPaneEntriesByAgent?.[agent.id] ?? [])
      .filter((entry) => entryBelongsToAgent(agent, entry))
    let historyPage = await options.loadHistoryPage(agent.id, null)
    let resolvedHistoryEntries = options.hydrateEntries(historyPage.entries)
    const currentRenderableCount = countRenderablePaneEntries(currentPaneEntries)
    const requestedHistoryCursorKeys = new Set<string>([historyCursorKey(null)])
    while (
      !agentHasPromptWork
      && historyPage.nextCursor
      && currentRenderableCount > countRenderablePaneEntries(resolvedHistoryEntries)
    ) {
      const cursorKey = historyCursorKey(historyPage.nextCursor)
      if (requestedHistoryCursorKeys.has(cursorKey)) {
        historyPage = { ...historyPage, nextCursor: null }
        break
      }
      requestedHistoryCursorKeys.add(cursorKey)
      historyPage = await options.loadHistoryPage(agent.id, historyPage.nextCursor)
      resolvedHistoryEntries = prependHistoryEntriesWithoutDuplicates(
        options.hydrateEntries(historyPage.entries),
        resolvedHistoryEntries,
      )
    }

    const availableTurnIds = new Set(
      resolvedHistoryEntries
        .map((entry) => entry.turnId)
        .filter((turnId): turnId is number => typeof turnId === "number"),
    )
    const collapsedTurnIds = options.preserveCollapsedTurnIds
      ? [...new Set(options.collapsedTurnIdsByAgent[agent.id] ?? [])]
      : (options.collapsedTurnIdsByAgent[agent.id] ?? []).filter((turnId) => availableTurnIds.has(turnId))
    if (collapsedTurnIds.length > 0) {
      collapsedTurnIdsByAgent[agent.id] = collapsedTurnIds
    }

    let nextPaneEntries = options.reindexEntries(
      options.applyCollapsedTurns(
        options.collapseHistoricalTurns(
          resolvedHistoryEntries,
          true,
        ),
        collapsedTurnIds,
      ),
      0,
    )
    nextPaneEntries = preserveLoadedHistoryBlobs({
      refreshedEntries: nextPaneEntries,
      currentEntries: currentPaneEntries,
      collapsedTurnIds,
      applyCollapsedTurns: options.applyCollapsedTurns,
      reindexEntries: options.reindexEntries,
    })
    if (agentHasPromptWork && shouldPreferCurrentPaneEntries(currentPaneEntries, nextPaneEntries)) {
      nextPaneEntries = currentPaneEntries.map((entry) => ({ ...entry }))
    }
    paneEntries[agent.id] = nextPaneEntries
    previews[agent.id] = options.formatPreview(nextPaneEntries)

    if (agent.id === visibleAgentId) {
      visibleEntries = nextPaneEntries
      visibleCursor = historyPage.nextCursor
    }
  }

  return {
    paneEntries,
    previews,
    collapsedTurnIdsByAgent,
    visibleAgentId,
    visibleEntries,
    visibleCursor,
  }
}
