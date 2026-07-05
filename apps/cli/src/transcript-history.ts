import {
  applyHistoryTranscriptDeferral,
  hydrateSessionHistoryTranscriptEntries,
  markDeferredHistoryTranscriptEntries,
  mergePrependedHistoryTranscriptFragments,
  previewLineForHistoryTranscriptEntry,
  stitchPrependedHistoryTranscript,
} from "@arroba/kernel-client/session-history-transcript"
import { mergeAdjacentSessionHistoryPageEntries } from "@arroba/kernel-client/session-history-page-entries"
import type { SessionHistoryEntry, SessionHistoryPageEntry, TranscriptEntry } from "./cli-types.js"

export function applyHistoryDeferral(entry: TranscriptEntry) {
  return applyHistoryTranscriptDeferral(entry)
}

export function markDeferredHistoryEntries(items: TranscriptEntry[]) {
  return markDeferredHistoryTranscriptEntries(items)
}

export function mergePrependedHistoryFragments(older: TranscriptEntry, newer: TranscriptEntry): TranscriptEntry {
  return mergePrependedHistoryTranscriptFragments(older, newer)
}

export function stitchPrependedHistory(olderEntries: TranscriptEntry[], currentEntries: TranscriptEntry[]) {
  return stitchPrependedHistoryTranscript(olderEntries, currentEntries)
}

export function mergeAdjacentHistoryPageEntries(historyEntries: SessionHistoryPageEntry[]) {
  return mergeAdjacentSessionHistoryPageEntries(historyEntries)
}

export function hydrateTranscriptEntries(
  historyEntries: SessionHistoryPageEntry[],
  hydrateOptions: { promptId?: string | null } = {},
): TranscriptEntry[] {
  return hydrateSessionHistoryTranscriptEntries(historyEntries, hydrateOptions) as TranscriptEntry[]
}

export function previewLineForHistoryEntry(entry: SessionHistoryEntry) {
  return previewLineForHistoryTranscriptEntry(entry)
}
