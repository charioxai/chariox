export type TranscriptEntryStateEntry = {
  readonly id: number
  readonly role: string
  readonly text: string
  readonly turnId?: number | null
  readonly promptId?: string | null
}

export function trimSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text
}

export function reindexTranscriptEntries<TEntry extends { readonly id: number }>(
  entries: readonly TEntry[],
  startingId: number,
): TEntry[] {
  return entries.map((entry, index) => ({
    ...entry,
    id: startingId + index + 1,
  }))
}

export function computeCurrentTranscriptTurnId<TEntry extends Pick<TranscriptEntryStateEntry, "role" | "turnId">>(
  entries: readonly TEntry[],
): number | null {
  return entries.reduce<number | null>((latest, entry) => {
    if (!entry || entry.role !== "user" || entry.turnId === undefined || entry.turnId === null) {
      return latest
    }
    return entry.turnId
  }, null)
}

export function computeNextTranscriptTurnId<TEntry extends Pick<TranscriptEntryStateEntry, "turnId">>(
  entries: readonly TEntry[],
): number {
  return entries.reduce((max, entry) => Math.max(max, entry?.turnId ?? 0), 0) + 1
}

export function computeNextTranscriptEntryId<TEntry extends Pick<TranscriptEntryStateEntry, "id">>(
  entries: readonly TEntry[],
): number {
  return entries.reduce((max, entry) => Math.max(max, entry.id), 0) + 1
}

export function transcriptHasTrailingUserPrompt<TEntry extends Pick<TranscriptEntryStateEntry, "role" | "text" | "promptId">>(
  entries: readonly TEntry[],
  text: string,
  promptId?: string | null,
): boolean {
  const lastEntry = entries.at(-1)
  if (lastEntry?.role !== "user") {
    return false
  }
  if (lastEntry.promptId && promptId) {
    return lastEntry.promptId === promptId
  }
  return trimSingleTrailingNewline(lastEntry.text) === trimSingleTrailingNewline(text)
}

export function createNextTranscriptEntry<
  TEntry extends TranscriptEntryStateEntry,
  TDraft extends Omit<TEntry, "id">,
>(
  currentEntries: readonly TEntry[],
  entry: TDraft,
): TEntry {
  const nextEntry = {
    id: computeNextTranscriptEntryId(currentEntries),
    ...entry,
  } as unknown as TEntry
  if (nextEntry.turnId === undefined) {
    const activeTurnId = computeCurrentTranscriptTurnId(currentEntries)
    if (activeTurnId !== null) {
      return {
        ...nextEntry,
        turnId: activeTurnId,
      }
    }
  }
  return nextEntry
}
