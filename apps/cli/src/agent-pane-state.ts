import {
  countRenderablePaneEntries,
  entryBelongsToAgent,
  focusedAgentIdForAgentPaneSession,
  preserveLoadedHistoryBlobs,
  prependHistoryEntriesWithoutDuplicates,
  selectCurrentAgentPaneEntries as sharedSelectCurrentAgentPaneEntries,
  shouldPreferCurrentPaneEntries,
  shouldRefreshAgentPanesForSessionChange as sharedShouldRefreshAgentPanesForSessionChange,
  trimAgentPaneEntries as sharedTrimAgentPaneEntries,
  type AgentPaneExternalProviderImport,
  type AgentPaneSession,
} from "@arroba/kernel-client/agent-pane-state"

export type { AgentPaneExternalProviderImport, AgentPaneSession }

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
  return sharedSelectCurrentAgentPaneEntries(options)
}

export function shouldRefreshAgentPanesForSessionChange<TAgent extends { id: string }>(options: {
  previousAgents: readonly TAgent[]
  nextAgents: readonly TAgent[]
  splitAgentResponseMode: boolean
  currentFocusedAgentId: string | null
  nextFocusedAgentId: string | null
}): boolean {
  return sharedShouldRefreshAgentPanesForSessionChange(options)
}

export function trimAgentPaneEntries<TEntry extends { text: string; mergeKey?: string }>(options: {
  entries: TEntry[]
  maxEntries: number
  maxChars: number
  onTrimmedMergeKey?: (mergeKey: string) => void
}) {
  return sharedTrimAgentPaneEntries(options)
}

function historyCursorKey(cursor: unknown): string {
  return JSON.stringify(cursor)
}

export async function refreshAgentPaneState<
  TAgent extends { id: string; external_provider_import?: AgentPaneExternalProviderImport | null },
  THistoryEntry,
  TEntry extends {
    role: string
    text: string
    turnId?: number
    source?: string | null
    externalProvider?: string | null
    externalProviderSessionId?: string | null
    externalProviderTurnId?: string | null
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
    const currentPaneEntries = (options.currentPaneEntriesByAgent?.[agent.id] ?? [])
      .filter((entry) => entryBelongsToAgent(agent, entry))
    let historyPage = await options.loadHistoryPage(agent.id, null)
    let resolvedHistoryEntries = options.hydrateEntries(historyPage.entries)
    const currentRenderableCount = countRenderablePaneEntries(currentPaneEntries)
    const requestedHistoryCursorKeys = new Set<string>([historyCursorKey(null)])
    while (
      !options.hasPromptWork
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
      visibleCursor = historyPage.nextCursor
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
