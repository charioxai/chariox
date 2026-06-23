export type AgentPaneSession<TAgent extends { id: string }> = {
  agents: TAgent[]
  focused_agent_id: string | null
}

export type AgentPaneRefreshResult<TEntry, TCursor> = {
  paneEntries: Record<string, TEntry[]>
  previews: Record<string, string>
  expandedTurnIdsByAgent: Record<string, number[]>
  visibleAgentId: string | null
  visibleEntries: TEntry[]
  visibleCursor: TCursor | null
}

export function selectCurrentAgentPaneEntries<TEntry extends object>(options: {
  agentId: string
  visibleAgentId: string | null
  visibleEntries: readonly TEntry[]
  paneEntriesByAgent: Record<string, TEntry[]>
}) {
  if (options.agentId === options.visibleAgentId) {
    return options.visibleEntries.map((entry) => ({ ...entry }))
  }
  return (options.paneEntriesByAgent[options.agentId] ?? []).map((entry) => ({ ...entry }))
}

export function shouldRefreshAgentPanesForSessionChange<TAgent extends { id: string }>(options: {
  previousAgents: readonly TAgent[]
  nextAgents: readonly TAgent[]
  splitAgentResponseMode: boolean
  currentFocusedAgentId: string | null
  nextFocusedAgentId: string | null
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

function countRenderablePaneEntries<TEntry extends { role: string }>(entries: readonly TEntry[]) {
  return entries.filter((entry) => entry.role !== "turn_toggle").length
}

function totalPaneTextLength<TEntry extends { text: string }>(entries: readonly TEntry[]) {
  return entries.reduce((sum, entry) => sum + entry.text.length, 0)
}

function shouldPreferCurrentPaneEntries<TEntry extends { role: string; text: string }>(
  currentEntries: readonly TEntry[],
  refreshedEntries: readonly TEntry[],
) {
  if (currentEntries.length === 0) {
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

export function trimAgentPaneEntries<TEntry extends { text: string; mergeKey?: string }>(options: {
  entries: TEntry[]
  maxEntries: number
  maxChars: number
  onTrimmedMergeKey?: (mergeKey: string) => void
}) {
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

export async function refreshAgentPaneState<
  TAgent extends { id: string },
  THistoryEntry,
  TEntry extends {
    role: string
    text: string
    turnId?: number
    historyBlobId?: string
    historyBlobAgentId?: string
    historyBlobSourceId?: string
    historyBlobSourceAgentId?: string
    historyBlobLoaded?: boolean
  },
  TCursor,
>(options: {
  session: AgentPaneSession<TAgent>
  hasPromptWork: boolean
  expandedTurnIdsByAgent: Record<string, number[]>
  currentPaneEntriesByAgent?: Record<string, TEntry[]>
  resolveVisibleAgentId: (agents: readonly TAgent[], focusedAgentId: string | null) => string | null
  loadHistoryPage: (agentId: string, cursor: TCursor | null) => Promise<{ entries: THistoryEntry[]; nextCursor: TCursor | null }>
  hydrateEntries: (entries: THistoryEntry[]) => TEntry[]
  stitchPrependedHistory: (olderEntries: TEntry[], currentEntries: TEntry[]) => TEntry[]
  collapseHistoricalTurns: (entries: TEntry[], keepLatestExpanded: boolean) => TEntry[]
  applyExpandedTurns: (entries: TEntry[], expandedTurnIds: readonly number[]) => TEntry[]
  reindexEntries: (entries: TEntry[], startingId: number) => TEntry[]
  formatPreview: (entries: TEntry[]) => string
  preserveExpandedTurnIds?: boolean
}): Promise<AgentPaneRefreshResult<TEntry, TCursor>> {
  const previews: Record<string, string> = {}
  const paneEntries: Record<string, TEntry[]> = {}
  const expandedTurnIdsByAgent: Record<string, number[]> = {}
  const visibleAgentId = options.resolveVisibleAgentId(
    options.session.agents,
    focusedAgentIdForAgentPaneSession(options.session),
  )
  let visibleEntries: TEntry[] = []
  let visibleCursor: TCursor | null = null

  for (const agent of options.session.agents) {
    const currentPaneEntries = options.currentPaneEntriesByAgent?.[agent.id] ?? []
    const historyPage = await options.loadHistoryPage(agent.id, null)
    let resolvedHistoryEntries = options.hydrateEntries(historyPage.entries)
    let nextResolvedCursor = historyPage.nextCursor
    while (nextResolvedCursor !== null) {
      const olderPage = await options.loadHistoryPage(agent.id, nextResolvedCursor)
      resolvedHistoryEntries = options.stitchPrependedHistory(
        options.hydrateEntries(olderPage.entries),
        resolvedHistoryEntries,
      )
      nextResolvedCursor = olderPage.nextCursor
    }

    const availableTurnIds = new Set(
      resolvedHistoryEntries
        .map((entry) => entry.turnId)
        .filter((turnId): turnId is number => typeof turnId === "number"),
    )
    const expandedTurnIds = options.preserveExpandedTurnIds
      ? [...new Set(options.expandedTurnIdsByAgent[agent.id] ?? [])]
      : (options.expandedTurnIdsByAgent[agent.id] ?? []).filter((turnId) => availableTurnIds.has(turnId))
    if (expandedTurnIds.length > 0) {
      expandedTurnIdsByAgent[agent.id] = expandedTurnIds
    }

    let nextPaneEntries = options.reindexEntries(
      options.applyExpandedTurns(
        options.collapseHistoricalTurns(
          resolvedHistoryEntries,
          true,
        ),
        expandedTurnIds,
      ),
      0,
    )
    nextPaneEntries = preserveLoadedHistoryBlobs({
      refreshedEntries: nextPaneEntries,
      currentEntries: currentPaneEntries,
      expandedTurnIds,
      applyExpandedTurns: options.applyExpandedTurns,
      reindexEntries: options.reindexEntries,
    })
    if (options.hasPromptWork && shouldPreferCurrentPaneEntries(currentPaneEntries, nextPaneEntries)) {
      nextPaneEntries = currentPaneEntries.map((entry) => ({ ...entry }))
    }
    paneEntries[agent.id] = nextPaneEntries
    previews[agent.id] = options.formatPreview(nextPaneEntries)

    if (agent.id === visibleAgentId) {
      visibleEntries = nextPaneEntries
      visibleCursor = null
    }
  }

  return {
    paneEntries,
    previews,
    expandedTurnIdsByAgent,
    visibleAgentId,
    visibleEntries,
    visibleCursor,
  }
}

function preserveLoadedHistoryBlobs<TEntry extends {
  text: string
  role: string
  turnId?: number
  historyBlobId?: string
  historyBlobAgentId?: string
  historyBlobSourceId?: string
  historyBlobSourceAgentId?: string
  historyBlobLoaded?: boolean
}>(options: {
  refreshedEntries: TEntry[]
  currentEntries: readonly TEntry[]
  expandedTurnIds: readonly number[]
  applyExpandedTurns: (entries: TEntry[], expandedTurnIds: readonly number[]) => TEntry[]
  reindexEntries: (entries: TEntry[], startingId: number) => TEntry[]
}) {
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

function historyBlobSourceKey(agentId: string | null | undefined, blobId: string, turnId: number | undefined) {
  return `${agentId ?? ""}:${blobId}:${turnId ?? ""}`
}

function focusedAgentIdForAgentPaneSession<TAgent extends { id: string }>(
  session: AgentPaneSession<TAgent>,
): string | null {
  const focusedAgentId = session.focused_agent_id
  if (focusedAgentId && session.agents.some((agent) => agent.id === focusedAgentId)) {
    return focusedAgentId
  }
  return session.agents[0]?.id ?? null
}
