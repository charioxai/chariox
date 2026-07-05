import type {
  AgentInstance,
  RuntimeSession,
  SessionHistoryCursorState,
  SessionHistoryOutlineCursor,
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
import { focusedAgentIdForSession, sessionHasPromptWork } from "./session-state.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries } from "@arroba/kernel-client/transcript-entry-state"

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
    cursor: SessionHistoryOutlineCursor | null,
  ) => Promise<{ entries: TranscriptEntry[]; nextCursor: SessionHistoryOutlineCursor | null }>
  pruneAuxiliaryAgentPanes: (session: RuntimeSession) => void
  setExpandedTurnIdsByAgent: (expandedTurnIdsByAgent: Record<string, number[]>) => void
  setAgentPanePreviews: (previews: Record<string, string>) => void
  setAgentPaneEntries: (entries: Record<string, TranscriptEntry[]>) => void
  setNextHistoryCursor: (cursor: SessionHistoryCursorState) => void
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
      nextFocusedAgentId: focusedAgentIdForSession(nextSession),
    })
  }

  const refresh = async (session: RuntimeSession) => {
    const nextPaneState = await refreshAgentPaneState<
      AgentInstance,
      TranscriptEntry,
      TranscriptEntry,
      SessionHistoryOutlineCursor
    >({
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
    deps.setNextHistoryCursor(
      nextPaneState.visibleAgentId && nextPaneState.visibleCursor
        ? { agentId: nextPaneState.visibleAgentId, cursor: nextPaneState.visibleCursor }
        : null,
    )
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
