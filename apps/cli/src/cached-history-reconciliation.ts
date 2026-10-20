import { reindexTranscriptEntries } from "@chariox/kernel-client/transcript-entry-state"
// MP-08 / MP-10 / MP-11: reconcile presentation only after the kernel confirms agent membership and sequence.
import type { BootstrapDeferredState, RuntimeSession, TranscriptEntry } from "./cli-types.js"
import type { CliCachedView } from "./client-view-cache.js"
type AttachedHistory = Awaited<NonNullable<BootstrapDeferredState["attachedHistory"]>>
export function historyEntrySequence(entry: TranscriptEntry): number[] {
  return [entry.historyEntryIndex, ...(entry.historySequenceRanges ?? []).flat()].filter((value): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value < Number.MAX_SAFE_INTEGER)
}
export function reconcileCachedHistory(cached: CliCachedView | null, session: RuntimeSession, history: AttachedHistory): { history: AttachedHistory; discard: boolean } {
  if (!cached || cached.sessionId !== session.id || history.sessionId !== session.id
    || !session.agents.some(agent => agent.id === cached.agentId)) return { history, discard: false }
  const current = history.agentEntries[cached.agentId]
  if (!current) return { history, discard: false }
  const sequence = Math.max(0, ...current.flatMap(historyEntrySequence))
  if (cached.sequence > sequence) return { history, discard: true }
  // A null server cursor confirms there is no earlier visible history to restore.
  if (history.visibleAgentId !== cached.agentId || !history.nextHistoryCursor) return { history, discard: false }
  const first = Math.min(...current.flatMap(historyEntrySequence))
  if (!Number.isFinite(first)) return { history, discard: false }
  // A missing interval must still be fetched through the server cursor.
  const currentSequences = new Set(current.flatMap(historyEntrySequence))
  if (!cached.entries.some(entry => historyEntrySequence(entry).some(sequence => currentSequences.has(sequence)))) return { history, discard: false }
  const older = cached.entries.filter(entry => {
    const sequences = historyEntrySequence(entry)
    return sequences.length > 0 && Math.max(...sequences) < first
  })
  if (!older.length) return { history, discard: false }
  const oldTurns = new Set(older.map(entry => entry.turnId))
  const included = new Set(older)
  const prefix = cached.entries.filter(entry => included.has(entry)
    || entry.role === "turn_toggle" && entry.turnId != null && oldTurns.has(entry.turnId))
    .map(entry => { const copy = { ...entry, historyBlobLoading: false }; delete copy.queuedPrompt; return copy })
  const entries = reindexTranscriptEntries([...prefix, ...current], 0)
  return { history: { ...history, agentEntries: { ...history.agentEntries, [cached.agentId]: entries },
    historyEntries: entries,
    nextHistoryCursor: { agentId: cached.agentId, cursor: { before_sequence: Math.min(...prefix.flatMap(historyEntrySequence)) } } }, discard: false }
}
