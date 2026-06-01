import type {
  AgentInstance,
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  refreshAgentPaneState,
  shouldRefreshAgentPanesForSessionChange,
} from "./agent-pane-state.js"
import {
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "./response-panes.js"
import { sessionHasPromptWork } from "./session-state.js"
import {
  stitchPrependedHistory,
} from "./transcript-history.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

type AgentPaneRefreshControllerDeps = {
  getCurrentAgents: () => readonly AgentInstance[]
  getFocusedAgentId: () => string | null
  getExpandedTurnIdsByAgent: () => Record<string, number[]>
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  splitAgentResponseMode: () => boolean
  maxAgentsPerScreen: () => number
  loadHistoryPage: (
    sessionId: string,
    agentId: string,
    cursor: null,
  ) => Promise<{ entries: TranscriptEntry[]; nextCursor: null }>
  pruneAuxiliaryAgentPanes: (session: RuntimeSession) => void
  setExpandedTurnIdsByAgent: (expandedTurnIdsByAgent: Record<string, number[]>) => void
  setAgentPanePreviews: (previews: Record<string, string>) => void
  setAgentPaneEntries: (entries: Record<string, TranscriptEntry[]>) => void
  setNextHistoryCursor: (cursor: null) => void
  applyExpandedTurns: (entries: TranscriptEntry[], expandedTurnIds: readonly number[]) => TranscriptEntry[]
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string | null) => void
  applyResponseLayout: () => void
  rebuildAuxiliaryAgentPane: (agentId: string) => void
}

export function createAgentPaneRefreshController(
  deps: AgentPaneRefreshControllerDeps,
) {
  const shouldRefreshForSessionChange = (nextSession: RuntimeSession) => {
    return shouldRefreshAgentPanesForSessionChange({
      previousAgents: deps.getCurrentAgents(),
      nextAgents: nextSession.agents,
      splitAgentResponseMode: deps.splitAgentResponseMode(),
      currentFocusedAgentId: deps.getFocusedAgentId(),
      nextFocusedAgentId: nextSession.focused_agent_id ?? nextSession.agents[0]?.id ?? null,
    })
  }

  const refresh = async (session: RuntimeSession) => {
    const nextPaneState = await refreshAgentPaneState<AgentInstance, TranscriptEntry, TranscriptEntry, null>({
      session,
      hasPromptWork: sessionHasPromptWork(session),
      expandedTurnIdsByAgent: deps.getExpandedTurnIdsByAgent(),
      currentPaneEntriesByAgent: Object.fromEntries(
        session.agents.map((agent) => [agent.id, deps.currentAgentPaneEntries(agent.id)]),
      ),
      resolveVisibleAgentId: (agents, focusedAgentId) =>
        selectResponsePaneAgents(
          agents,
          focusedAgentId,
          deps.splitAgentResponseMode(),
          deps.maxAgentsPerScreen(),
        ).visibleTranscriptAgentId,
      loadHistoryPage: (agentId, cursor) => deps.loadHistoryPage(session.id, agentId, cursor),
      hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
      stitchPrependedHistory,
      collapseHistoricalTurns: (entries) => entries,
      applyExpandedTurns: deps.applyExpandedTurns,
      reindexEntries: reindexTranscriptEntries,
      formatPreview: formatTranscriptPreview,
      preserveExpandedTurnIds: true,
    })

    deps.pruneAuxiliaryAgentPanes(session)
    deps.setExpandedTurnIdsByAgent(nextPaneState.expandedTurnIdsByAgent)
    deps.setAgentPanePreviews(nextPaneState.previews)
    deps.setAgentPaneEntries(nextPaneState.paneEntries)
    deps.setNextHistoryCursor(nextPaneState.visibleCursor)
    const visibleEntries = nextPaneState.visibleAgentId
      ? nextPaneState.paneEntries[nextPaneState.visibleAgentId]
      : nextPaneState.visibleEntries
    deps.replaceTranscriptEntries(
      visibleEntries?.map((entry) => ({ ...entry })) ?? [],
      nextPaneState.visibleAgentId,
    )
    deps.applyResponseLayout()
    if (!deps.splitAgentResponseMode()) {
      return
    }
    for (const agentId of splitPaneAuxiliaryAgentIds(
      session.agents,
      session.focused_agent_id,
      true,
      deps.maxAgentsPerScreen(),
    )) {
      deps.rebuildAuxiliaryAgentPane(agentId)
    }
  }

  return {
    refresh,
    shouldRefreshForSessionChange,
  }
}
