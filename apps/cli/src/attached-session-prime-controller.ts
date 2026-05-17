import type {
  RuntimeSession,
  SessionHistoryPage,
  TranscriptEntry,
} from "./cli-types.js"
import type { PromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { selectResponsePaneAgents } from "./response-panes.js"
import { hydrateTranscriptEntries } from "./transcript-history.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

export type AttachedSessionPrimeControllerDeps = {
  promptHistoryHydrationController: Pick<PromptHistoryHydrationController, "begin" | "loadAndApply">
  splitAgentResponseMode: () => boolean
  maxAgentsPerScreen: () => number
  loadVisibleAgentHistory: (sessionId: string, agentId: string) => Promise<SessionHistoryPage>
  setAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  setAgentPanePreview: (agentId: string, preview: string) => void
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string | null) => void
  setNextHistoryCursor: (cursor: SessionHistoryPage["next_cursor"]) => void
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

    const historyPage = await deps.loadVisibleAgentHistory(session.id, visibleAgentId)
    await deps.promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
    const preparedEntries = reindexTranscriptEntries(
      hydrateTranscriptEntries(historyPage.entries),
      0,
    )

    deps.setAgentPaneEntries(visibleAgentId, cloneTranscriptEntries(preparedEntries))
    deps.setAgentPanePreview(visibleAgentId, formatTranscriptPreview(preparedEntries))
    deps.replaceTranscriptEntries(cloneTranscriptEntries(preparedEntries), visibleAgentId)
    deps.setNextHistoryCursor(historyPage.next_cursor)
  }

  return {
    prime,
  }
}

function cloneTranscriptEntries(entries: TranscriptEntry[]) {
  return entries.map((entry) => ({ ...entry }))
}
