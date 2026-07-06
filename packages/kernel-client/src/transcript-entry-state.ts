import type { TranscriptEntry as KernelTranscriptEntry } from "./kernel-types.js"

type TranscriptEntryStateKernelFields = Pick<KernelTranscriptEntry, "id" | "text"> & {
  readonly turnId?: KernelTranscriptEntry["turnId"] | null
}

export type TranscriptEntryStateEntry = {
  readonly id: TranscriptEntryStateKernelFields["id"]
  readonly role: string
  readonly text: TranscriptEntryStateKernelFields["text"]
  readonly turnId?: TranscriptEntryStateKernelFields["turnId"]
  readonly promptId?: string | null
}

export type TranscriptRetentionSlice<TEntry extends Pick<TranscriptEntryStateEntry, "text">> = {
  readonly removed: TEntry[]
  readonly kept: TEntry[]
  readonly changed: boolean
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

export function transcriptRetentionSlice<TEntry extends Pick<TranscriptEntryStateEntry, "text">>(
  entries: readonly TEntry[],
  options: {
    readonly maxEntries: number
    readonly maxChars: number
  },
): TranscriptRetentionSlice<TEntry> {
  const currentEntries = entries.map((entry) => ({ ...entry })) as TEntry[]
  const maxEntries = finiteIntegerOrZero(options.maxEntries)
  const maxChars = finiteIntegerOrZero(options.maxChars)
  let totalChars = currentEntries.reduce((sum, entry) => sum + entry.text.length, 0)
  let removeCount = 0

  while (
    currentEntries.length - removeCount > maxEntries
    || (totalChars > maxChars && removeCount < currentEntries.length - 1)
  ) {
    totalChars -= currentEntries[removeCount]?.text.length ?? 0
    removeCount += 1
  }

  if (removeCount === 0) {
    return {
      removed: [],
      kept: currentEntries,
      changed: false,
    }
  }

  return {
    removed: currentEntries.slice(0, removeCount),
    kept: currentEntries.slice(removeCount),
    changed: true,
  }
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
  options: {
    readonly nextEntryId?: number
    readonly currentTurnId?: number | null
  } = {},
): TEntry {
  const nextEntry = {
    id: options.nextEntryId ?? computeNextTranscriptEntryId(currentEntries),
    ...entry,
  } as unknown as TEntry
  if (nextEntry.turnId === undefined) {
    const activeTurnId = options.currentTurnId !== undefined
      ? options.currentTurnId
      : computeCurrentTranscriptTurnId(currentEntries)
    if (activeTurnId !== null) {
      return {
        ...nextEntry,
        turnId: activeTurnId,
      }
    }
  }
  return nextEntry
}

function finiteIntegerOrZero(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0
}
