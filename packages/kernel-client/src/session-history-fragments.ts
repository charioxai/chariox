export type SessionHistoryFragmentRange = {
  readonly entry_index?: number | null | undefined
  readonly fragment_start?: number | null | undefined
  readonly fragment_end?: number | null | undefined
}

export type TranscriptHistoryFragmentRange = {
  readonly historyEntryIndex?: number | null | undefined
  readonly historyFragmentStart?: number | null | undefined
  readonly historyFragmentEnd?: number | null | undefined
}

export type TranscriptHistoryDeferrableEntry = TranscriptHistoryFragmentRange & {
  historyDeferred?: boolean | undefined
}

export function sessionHistoryFragmentsAreAdjacent(
  older: SessionHistoryFragmentRange | null | undefined,
  newer: SessionHistoryFragmentRange | null | undefined,
): boolean {
  return typeof older?.entry_index === "number"
    && typeof newer?.entry_index === "number"
    && older.entry_index === newer.entry_index
    && typeof older.fragment_end === "number"
    && typeof newer.fragment_start === "number"
    && older.fragment_end === newer.fragment_start
}

export function transcriptHistoryFragmentsAreAdjacent(
  older: TranscriptHistoryFragmentRange | null | undefined,
  newer: TranscriptHistoryFragmentRange | null | undefined,
): boolean {
  return typeof older?.historyEntryIndex === "number"
    && typeof newer?.historyEntryIndex === "number"
    && older.historyEntryIndex === newer.historyEntryIndex
    && typeof older.historyFragmentEnd === "number"
    && typeof newer.historyFragmentStart === "number"
    && older.historyFragmentEnd === newer.historyFragmentStart
}

export function transcriptHistoryFragmentShouldDefer(
  entry: TranscriptHistoryFragmentRange,
): boolean {
  return typeof entry.historyFragmentStart === "number" && entry.historyFragmentStart > 0
}

export function applyTranscriptHistoryDeferral<T extends TranscriptHistoryDeferrableEntry>(entry: T): T {
  if (transcriptHistoryFragmentShouldDefer(entry)) {
    entry.historyDeferred = true
  } else {
    delete entry.historyDeferred
  }
  return entry
}

export function markDeferredTranscriptHistoryEntries<T extends TranscriptHistoryDeferrableEntry>(items: T[]): T[] {
  if (items.length === 0) {
    return items
  }

  return items.map((entry, index) => {
    if (index === 0) {
      return applyTranscriptHistoryDeferral({ ...entry })
    }
    if (!entry.historyDeferred) {
      return entry
    }
    const next = { ...entry }
    delete next.historyDeferred
    return next
  })
}
