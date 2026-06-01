import type {
  RuntimeSession,
  SessionHistoryOutline,
  TranscriptEntry,
} from "./cli-types.js"
import type { PromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { selectResponsePaneAgents } from "./response-panes.js"
import { hydrateOutlineAgentEntries } from "./session-history-outline.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

export type AttachedSessionPrimeControllerDeps = {
  promptHistoryHydrationController: Pick<PromptHistoryHydrationController, "begin" | "loadAndApply">
  splitAgentResponseMode: () => boolean
  maxAgentsPerScreen: () => number
  loadSessionHistoryOutline: (sessionId: string, agentIds: readonly string[]) => Promise<SessionHistoryOutline>
  setAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  setAgentPanePreview: (agentId: string, preview: string) => void
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string | null) => void
  setNextHistoryCursor: (cursor: null) => void
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
    const entriesByAgent = new Map(outline.agents.map((agent) => [
      agent.agent_id,
      reindexTranscriptEntries(hydrateOutlineAgentEntries(agent), 0),
    ]))
    for (const [agentId, entries] of entriesByAgent) {
      deps.setAgentPaneEntries(agentId, cloneTranscriptEntries(entries))
      deps.setAgentPanePreview(agentId, formatTranscriptPreview(entries))
    }
    const preparedEntries = entriesByAgent.get(visibleAgentId) ?? []

    deps.replaceTranscriptEntries(cloneTranscriptEntries(preparedEntries), visibleAgentId)
    deps.setNextHistoryCursor(null)
  }

  return {
    prime,
  }
}

function cloneTranscriptEntries(entries: TranscriptEntry[]) {
  return entries.map((entry) => ({ ...entry }))
}
