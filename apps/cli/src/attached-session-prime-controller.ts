import type {
  RuntimeSession,
  SessionHistoryCursorState,
  SessionHistoryOutline,
  TranscriptEntry,
} from "./cli-types.js"
import type { PromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
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
  promptHistoryHydrationController: Pick<PromptHistoryHydrationController, "begin" | "loadAndApply">
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
  // `owned` goes false once a newer selection took the view: nothing is written then.
  const prime = async (session: RuntimeSession, owned: () => boolean = () => true) => {
    const promptHistoryGeneration = deps.promptHistoryHydrationController.begin()
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
      await deps.promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
      return
    }

    const outline = await deps.loadSessionHistoryOutline(
      session.id,
      session.agents.map((agent) => agent.id),
    )
    if (!owned()) return
    await deps.promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
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
