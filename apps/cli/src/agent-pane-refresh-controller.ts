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
} from "@arroba/kernel-client/agent-pane-state"
import {
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "@arroba/kernel-client/response-pane-selection"
import { sessionFocusedAgentId } from "@arroba/kernel-client/session-runtime-transition"
import { sessionAgentIsBusy } from "@arroba/kernel-client/session-prompt-work"
import { formatTranscriptPreview } from "@arroba/kernel-client/session-history-preview"
import { reindexTranscriptEntries } from "@arroba/kernel-client/transcript-entry-state"

type AgentPaneRefreshControllerDeps = {
  getCurrentAgents: () => readonly AgentInstance[]
  getFocusedAgentId: () => string | null
  getCollapsedTurnIdsByAgent: () => Record<string, number[]>
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  splitAgentResponseMode: () => boolean
  maxAgentsPerScreen: () => number
  loadHistoryPage: (
    sessionId: string,
    agentId: string,
    cursor: SessionHistoryOutlineCursor | null,
  ) => Promise<{ entries: TranscriptEntry[]; nextCursor: SessionHistoryOutlineCursor | null }>
  pruneAuxiliaryAgentPanes: (session: RuntimeSession) => void
  setCollapsedTurnIdsByAgent: (collapsedTurnIdsByAgent: Record<string, number[]>) => void
  setAgentPanePreviews: (previews: Record<string, string>) => void
  setAgentPaneEntries: (entries: Record<string, TranscriptEntry[]>) => void
  setNextHistoryCursor: (cursor: SessionHistoryCursorState) => void
  applyCollapsedTurns: (entries: TranscriptEntry[], collapsedTurnIds: readonly number[]) => TranscriptEntry[]
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
      nextFocusedAgentId: sessionFocusedAgentId(nextSession),
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
      hasPromptWorkForAgent: (agent) => sessionAgentIsBusy(session, agent.id),
      collapsedTurnIdsByAgent: deps.getCollapsedTurnIdsByAgent(),
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
      applyCollapsedTurns: deps.applyCollapsedTurns,
      reindexEntries: reindexTranscriptEntries,
      formatPreview: formatTranscriptPreview,
      preserveCollapsedTurnIds: true,
    })

    deps.pruneAuxiliaryAgentPanes(session)
    deps.setCollapsedTurnIdsByAgent(nextPaneState.collapsedTurnIdsByAgent)
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
