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

// MP-08 / MP-10 / MP-11 H2. Kernel offsets count Rust chars (Unicode code points).
export type SessionHistoryMessageFragment = {
  readonly entry_index: number
  readonly fragment_start: number
  readonly fragment_end: number
  readonly total_chars: number
  readonly entry: {
    readonly kind: string
    readonly text: string
    readonly provider_run_id?: string | null
    readonly merge_key?: string
    readonly timestamp_ms?: number
  }
}

export function assembleSessionHistoryEntry(fragments: readonly SessionHistoryMessageFragment[]): string {
  if (!fragments.length) throw new Error("incomplete history entry")
  const ordered = [...fragments].sort((a, b) => a.fragment_start - b.fragment_start || b.fragment_end - a.fragment_end)
  const first = ordered[0]!
  const result: string[] = []
  for (const fragment of ordered) {
    const { entry_index, fragment_start: start, fragment_end: end, total_chars: total, entry } = fragment
    if (![entry_index, start, end, total].every(Number.isSafeInteger)
      || entry_index < 0 || start < 0 || end < start || end > total
      || entry_index !== first.entry_index || total !== first.total_chars
      || entry.kind !== first.entry.kind || entry.provider_run_id !== first.entry.provider_run_id
      || entry.merge_key !== first.entry.merge_key
      || entry.timestamp_ms !== first.entry.timestamp_ms) throw new Error("conflicting history fragment metadata")
    const chars = Array.from(entry.text)
    if (chars.length !== end - start) throw new Error("conflicting history fragment length")
    if (start > result.length || (start === result.length && !sessionHistoryFragmentsAreAdjacent(
      { entry_index, fragment_end: result.length }, fragment,
    ))) throw new Error("incomplete history fragment range")
    for (let index = 0; index < chars.length; index++) {
      const position = start + index
      if (position < result.length) {
        if (result[position] !== chars[index]) throw new Error("conflicting history fragment text")
      } else {
        result.push(chars[index]!)
      }
    }
  }
  if (result.length !== first.total_chars) throw new Error("incomplete history entry")
  return result.join("")
}

export function assembleSessionHistoryFinalMessage(
  turn: { readonly lifecycle: string; readonly summary?: SessionHistoryMessageFragment | null },
  entries: readonly SessionHistoryMessageFragment[],
): string {
  if (turn.lifecycle !== "completed") throw new Error("final message requires completed settlement")
  const summary = turn.summary
  if (!summary || summary.entry.kind !== "provider_output") throw new Error("missing final message summary identity")
  if (summary.fragment_start !== 0) throw new Error("incomplete final summary")
  if (summary.fragment_end === summary.total_chars) {
    const copies = entries.filter(item => item.entry_index === summary.entry_index && item.total_chars === summary.total_chars)
    return assembleSessionHistoryEntry([summary, ...copies])
  }

  // A kernel summary may combine several consecutive provider-output events.
  // Its preview is not an original fragment; do not append it to blob content.
  const groups = new Map<number, SessionHistoryMessageFragment[]>()
  for (const item of entries) {
    if (item.entry_index < summary.entry_index || item.entry.kind !== "provider_output"
      || item.entry.provider_run_id !== summary.entry.provider_run_id
      || item.entry.merge_key !== summary.entry.merge_key) continue
    const group = groups.get(item.entry_index) ?? []
    group.push(item)
    groups.set(item.entry_index, group)
  }
  if (!groups.has(summary.entry_index)) throw new Error("incomplete final message start")
  const firstGroup = groups.get(summary.entry_index)!
  if (firstGroup.every(item => item.total_chars === summary.total_chars)) firstGroup.push(summary)
  const text = [...groups.values()]
    .sort((a, b) => (a[0]!.entry.timestamp_ms ?? 0) - (b[0]!.entry.timestamp_ms ?? 0)
      || a[0]!.entry_index - b[0]!.entry_index)
    .map(assembleSessionHistoryEntry).join("")
  if (Array.from(text).length !== summary.total_chars) throw new Error("incomplete final message")
  if (!text.startsWith(summary.entry.text)) throw new Error("conflicting final summary preview")
  return text
}
