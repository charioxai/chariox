// MP-08 / MP-10 / MP-11: render accepted kernel state before the later terminal echo.
import type { RuntimeSession } from "./kernel-types.js"
import type { SessionHistoryTranscriptEntry } from "./session-history-transcript.js"
import { sessionAgentHasTurnWork } from "./session-prompt-work.js"
import { sessionProjectedPromptActivityForAgent, sessionPromptStateRecordForAgent } from "./session-agent-prompt-state.js"
import { computeNextTranscriptEntryId, computeNextTranscriptTurnId, createTranscriptUserPromptTurn, promptQueueItemTranscriptMetadata, transcriptHasTrailingUserPrompt } from "./transcript-entry-state.js"

export function syncActivePromptTranscriptEntriesForAgent<TEntry extends SessionHistoryTranscriptEntry>(
  entries: readonly TEntry[], session: RuntimeSession, agentId: string,
): { entries: TEntry[]; changed: boolean } {
  const unchanged = () => ({ entries: [...entries], changed: false })
  if (!sessionAgentHasTurnWork(session, agentId)) return unchanged()
  const prompt = sessionPromptStateRecordForAgent(session, agentId)?.active_prompt
  if (!prompt || prompt.target_agent_id !== agentId || prompt.status !== "Running") return unchanged()
  const activity = sessionProjectedPromptActivityForAgent(session, agentId)
  if (activity && activity !== "idle" && activity !== "not_found"
    && activity.activeTurnPromptId && activity.activeTurnPromptId !== prompt.id) return unchanged()
  if (transcriptHasTrailingUserPrompt(entries, prompt.prompt, prompt.id)) return unchanged()
  const turn = createTranscriptUserPromptTurn(prompt.prompt, computeNextTranscriptTurnId(entries), promptQueueItemTranscriptMetadata(prompt))
  const entry = {
    ...turn.entry,
    id: computeNextTranscriptEntryId(entries),
    ...(activity && activity !== "idle" && activity !== "not_found" && activity.activeTurnProviderRunId
      ? { providerRunId: activity.activeTurnProviderRunId } : {}),
    ...(prompt.attachments?.length ? { attachments: prompt.attachments.map(({ url, mime, filename }) => ({ url, mime, ...(filename ? { filename } : {}) })) } : {}),
  } as TEntry
  return { entries: [...entries, entry], changed: true }
}
