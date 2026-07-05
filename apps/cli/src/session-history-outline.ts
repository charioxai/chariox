import {
  hydrateSessionHistoryOutlineAgentEntries,
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
} from "@arroba/kernel-client/session-history-transcript"
import { sessionHistoryCursorForVisibleAgent } from "@arroba/kernel-client/session-history-outline"
import type {
  SessionHistoryBlobContent,
  SessionHistoryCursorState,
  SessionHistoryOutline,
  SessionHistoryOutlineAgent,
  TranscriptEntry,
} from "./cli-types.js"

export function hydrateOutlineAgentEntries(agent: SessionHistoryOutlineAgent): TranscriptEntry[] {
  return hydrateSessionHistoryOutlineAgentEntries(agent) as TranscriptEntry[]
}

export function historyCursorStateForVisibleAgent(
  outline: SessionHistoryOutline,
  visibleAgentId: string | null,
): SessionHistoryCursorState {
  return sessionHistoryCursorForVisibleAgent(outline, visibleAgentId)
}

export function replaceHistoryBlobPlaceholder(
  entries: TranscriptEntry[],
  entryId: number,
  content: SessionHistoryBlobContent,
  expandedTurnIds: readonly number[],
): TranscriptEntry[] {
  return replaceSessionHistoryBlobPlaceholder(entries, entryId, content, expandedTurnIds) as TranscriptEntry[]
}

export function markHistoryBlobLoading(
  entries: TranscriptEntry[],
  entryId: number,
  loading: boolean,
  error?: string | null,
): TranscriptEntry[] {
  return markSessionHistoryBlobLoading(entries, entryId, loading, error) as TranscriptEntry[]
}
