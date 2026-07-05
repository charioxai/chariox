import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  syncQueuedPromptTranscriptEntriesByAgent as sharedSyncQueuedPromptTranscriptEntriesByAgent,
  syncQueuedPromptTranscriptEntriesForAgent as sharedSyncQueuedPromptTranscriptEntriesForAgent,
} from "@arroba/kernel-client/queued-prompt-strip-state"
import { formatTranscriptPreview } from "@arroba/kernel-client/session-history-preview"

export type QueuedPromptTranscriptSyncResult = {
  entries: TranscriptEntry[]
  changed: boolean
}

export function syncQueuedPromptEntriesForAgent(
  entries: readonly TranscriptEntry[],
  session: RuntimeSession,
  agentId: string,
): QueuedPromptTranscriptSyncResult {
  return sharedSyncQueuedPromptTranscriptEntriesForAgent(entries, session, agentId)
}

export function syncQueuedPromptEntriesByAgent(
  entriesByAgent: Record<string, TranscriptEntry[]>,
  session: RuntimeSession,
): { entriesByAgent: Record<string, TranscriptEntry[]>; previews: Record<string, string>; changed: boolean } {
  const synced = sharedSyncQueuedPromptTranscriptEntriesByAgent(entriesByAgent, session)
  const previews: Record<string, string> = {}
  for (const agentId of synced.changedAgentIds) {
    previews[agentId] = formatTranscriptPreview(synced.entriesByAgent[agentId] ?? [])
  }
  return { entriesByAgent: synced.entriesByAgent, previews, changed: synced.changed }
}
