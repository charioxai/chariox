import type {
  RuntimeSession,
  SessionHistoryCursorState,
  SessionHistoryOutline,
  TranscriptEntry,
} from "./cli-types.js"
import type { PromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { selectResponsePaneAgents } from "@arroba/kernel-client/response-pane-selection"
import {
  sessionHistoryCursorForVisibleAgent,
} from "@arroba/kernel-client/session-history-outline"
import {
  hydrateSessionHistoryOutlineAgentEntries,
} from "@arroba/kernel-client/session-history-transcript"
import { formatTranscriptPreview } from "@arroba/kernel-client/session-history-preview"
import { reindexTranscriptEntries } from "@arroba/kernel-client/transcript-entry-state"
import { entryBelongsToAgent } from "@arroba/kernel-client/agent-pane-state"

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
  const prime = async (session: RuntimeSession) => {
    const promptHistoryGeneration = deps.promptHistoryHydrationController.begin()
    const visibleAgentId = selectResponsePaneAgents(
      session.agents,
      session.focused_agent_id,
      deps.splitAgentResponseMode(),
      deps.maxAgentsPerScreen(),
    ).visibleTranscriptAgentId

    if (!visibleAgentId) {
      deps.replaceTranscriptEntries([], null)
      deps.setNextHistoryCursor(null)
      await deps.promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
      return
    }

    const outline = await deps.loadSessionHistoryOutline(
      session.id,
      session.agents.map((agent) => agent.id),
    )
    await deps.promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
    const agentsById = new Map(session.agents.map((agent) => [agent.id, agent]))
    const entriesByAgent = new Map(outline.agents.map((agent) => {
      const sessionAgent = agentsById.get(agent.agent_id)
      const entries = hydrateSessionHistoryOutlineAgentEntries(agent)
        .filter((entry) => !sessionAgent || entryBelongsToAgent(sessionAgent, entry))
      return [
        agent.agent_id,
        reindexTranscriptEntries(entries, 0),
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
