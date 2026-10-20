import type {
  RuntimeSession,
  SessionHistoryCursorState,
  SessionHistoryOutline,
  TranscriptEntry,
} from "./cli-types.js"
import { selectResponsePaneAgents } from "@chariox/kernel-client/response-pane-selection"
import {
  sessionHistoryCursorForVisibleAgent,
} from "@chariox/kernel-client/session-history-outline"
import {
  hydrateSessionHistoryOutlineAgentEntries,
} from "@chariox/kernel-client/session-history-transcript"
import { formatTranscriptPreview } from "@chariox/kernel-client/session-history-preview"
import { reindexTranscriptEntries } from "@chariox/kernel-client/transcript-entry-state"

export type AttachedSessionPrimeControllerDeps = {
  splitAgentResponseMode: () => boolean
  maxAgentsPerScreen: () => number
  loadSessionHistoryOutline: (sessionId: string, agentIds: readonly string[]) => Promise<SessionHistoryOutline>
  setAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  setAgentPanePreview: (agentId: string, preview: string) => void
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string | null) => void
  setNextHistoryCursor: (cursor: SessionHistoryCursorState) => void
}

export function createAttachedSessionPrimeController(
  deps: AttachedSessionPrimeControllerDeps,
) {
  const prime = async (session: RuntimeSession, owned: () => boolean = () => true) => {
    // The attachment controller owns prompt-history generations. Transcript
    // priming must not invalidate its in-flight read after a room switch.
    const visibleAgentId = selectResponsePaneAgents(
      session.agents,
      session.focused_agent_id,
      deps.splitAgentResponseMode(),
      deps.maxAgentsPerScreen(),
    ).visibleTranscriptAgentId

    if (!visibleAgentId) {
      if (!owned()) return
      deps.replaceTranscriptEntries([], null)
      deps.setNextHistoryCursor(null)
      return
    }

    const outline = await deps.loadSessionHistoryOutline(
      session.id,
      session.agents.map((agent) => agent.id),
    )
    if (!owned()) return
    const entriesByAgent = new Map(outline.agents.map((agent) => {
      return [
        agent.agent_id,
        reindexTranscriptEntries(hydrateSessionHistoryOutlineAgentEntries(agent), 0),
      ]
    }))
    for (const [agentId, entries] of entriesByAgent) {
      deps.setAgentPaneEntries(agentId, cloneTranscriptEntries(entries))
      deps.setAgentPanePreview(agentId, formatTranscriptPreview(entries))
    }
    const preparedEntries = entriesByAgent.get(visibleAgentId) ?? []

    deps.replaceTranscriptEntries(cloneTranscriptEntries(preparedEntries), visibleAgentId)
    deps.setNextHistoryCursor(sessionHistoryCursorForVisibleAgent(outline, visibleAgentId))
  }

  return {
    prime,
  }
}

function cloneTranscriptEntries(entries: TranscriptEntry[]) {
  return entries.map((entry) => ({ ...entry }))
}
