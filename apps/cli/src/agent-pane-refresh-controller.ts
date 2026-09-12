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
} from "@chariox/kernel-client/agent-pane-state"
import {
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "@chariox/kernel-client/response-pane-selection"
import { sessionFocusedAgentId } from "@chariox/kernel-client/session-runtime-transition"
import { sessionAgentHasTurnWork } from "@chariox/kernel-client/session-prompt-work"
import { formatTranscriptPreview } from "@chariox/kernel-client/session-history-preview"
import { reindexTranscriptEntries } from "@chariox/kernel-client/transcript-entry-state"
import { retainRoomActivityNotices } from "./room-activity-notice-state.js"

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
  isCurrentSession: (sessionId: string) => boolean
}

export function createAgentPaneRefreshController(
  deps: AgentPaneRefreshControllerDeps,
) {
  const visibleAgentIdForSession = (session: RuntimeSession) =>
    selectResponsePaneAgents(
      session.agents,
      session.focused_agent_id,
      deps.splitAgentResponseMode(),
      deps.maxAgentsPerScreen(),
    ).visibleTranscriptAgentId

  const shouldRefreshForSessionChange = (nextSession: RuntimeSession) => {
    return shouldRefreshAgentPanesForSessionChange({
      previousAgents: deps.getCurrentAgents(),
      nextAgents: nextSession.agents,
      splitAgentResponseMode: deps.splitAgentResponseMode(),
      currentFocusedAgentId: deps.getFocusedAgentId(),
      nextFocusedAgentId: sessionFocusedAgentId(nextSession),
    })
  }

  const loadPaneState = async (
    session: RuntimeSession,
    agents: readonly AgentInstance[],
  ) => {
    const nextPaneState = await refreshAgentPaneState<
      AgentInstance,
      TranscriptEntry,
      TranscriptEntry,
      SessionHistoryOutlineCursor
    >({
      session: { ...session, agents: [...agents] },
      hasTurnWorkForAgent: (agent) => sessionAgentHasTurnWork(session, agent.id),
      collapsedTurnIdsByAgent: deps.getCollapsedTurnIdsByAgent(),
      currentPaneEntriesByAgent: Object.fromEntries(
        session.agents.map((agent) => [agent.id, deps.currentAgentPaneEntries(agent.id)]),
      ),
      resolveVisibleAgentId: () => visibleAgentIdForSession(session),
      loadHistoryPage: (agentId, cursor) => deps.loadHistoryPage(session.id, agentId, cursor),
      hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
      collapseHistoricalTurns: (entries) => entries,
      applyCollapsedTurns: deps.applyCollapsedTurns,
      reindexEntries: reindexTranscriptEntries,
      formatPreview: formatTranscriptPreview,
      preserveCollapsedTurnIds: true,
    })

    // Read current notices after history I/O: events may arrive while it loads.
    const paneEntries = Object.fromEntries(Object.entries(nextPaneState.paneEntries).map(([agentId, entries]) => [
      agentId, retainRoomActivityNotices(entries, deps.currentAgentPaneEntries(agentId), session.id),
    ]))
    return { ...nextPaneState, paneEntries,
      visibleEntries: nextPaneState.visibleAgentId ? paneEntries[nextPaneState.visibleAgentId] ?? [] : [],
      previews: Object.fromEntries(Object.entries(paneEntries).map(([id, entries]) => [id, formatTranscriptPreview(entries)])),
    }
  }

  const refresh = async (session: RuntimeSession) => {
    const nextPaneState = await loadPaneState(session, session.agents)
    if (!deps.isCurrentSession(session.id)) return

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

  const refreshAgentHistories = async (
    session: RuntimeSession,
    agentIds: readonly string[],
  ) => {
    const requestedAgentIds = new Set(agentIds)
    const requestedAgents = session.agents.filter((agent) => requestedAgentIds.has(agent.id))
    if (requestedAgents.length === 0) {
      return
    }

    const nextPaneState = await loadPaneState(session, requestedAgents)
    if (!deps.isCurrentSession(session.id)) {
      return
    }

    const currentPaneEntries = Object.fromEntries(
      session.agents.map((agent) => [agent.id, deps.currentAgentPaneEntries(agent.id)]),
    )
    const currentCollapsedTurnIds = deps.getCollapsedTurnIdsByAgent()
    const mergedCollapsedTurnIds = Object.fromEntries(
      session.agents.flatMap((agent) => {
        const turnIds = requestedAgentIds.has(agent.id)
          ? nextPaneState.collapsedTurnIdsByAgent[agent.id]
          : currentCollapsedTurnIds[agent.id]
        return turnIds && turnIds.length > 0 ? [[agent.id, [...turnIds]]] : []
      }),
    )
    const mergedPaneEntries = {
      ...currentPaneEntries,
      ...nextPaneState.paneEntries,
    }
    const mergedPreviews = Object.fromEntries(
      session.agents.map((agent) => [agent.id, formatTranscriptPreview(mergedPaneEntries[agent.id] ?? [])]),
    )

    deps.pruneAuxiliaryAgentPanes(session)
    deps.setCollapsedTurnIdsByAgent(mergedCollapsedTurnIds)
    deps.setAgentPanePreviews(mergedPreviews)
    deps.setAgentPaneEntries(mergedPaneEntries)

    const visibleAgentId = visibleAgentIdForSession(session)
    if (visibleAgentId && requestedAgentIds.has(visibleAgentId)) {
      deps.setNextHistoryCursor(
        nextPaneState.visibleCursor
          ? { agentId: visibleAgentId, cursor: nextPaneState.visibleCursor }
          : null,
      )
      deps.replaceTranscriptEntries(
        mergedPaneEntries[visibleAgentId]?.map((entry) => ({ ...entry })) ?? [],
        visibleAgentId,
      )
    }

    deps.applyResponseLayout()
    if (!deps.splitAgentResponseMode()) {
      return
    }
    const auxiliaryAgentIds = new Set(splitPaneAuxiliaryAgentIds(
      session.agents,
      session.focused_agent_id,
      true,
      deps.maxAgentsPerScreen(),
    ))
    for (const agent of requestedAgents) {
      if (auxiliaryAgentIds.has(agent.id)) {
        deps.rebuildAuxiliaryAgentPane(agent.id)
      }
    }
  }

  return {
    refresh,
    refreshAgentHistories,
    shouldRefreshForSessionChange,
  }
}
