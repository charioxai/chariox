import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"

export type TranscriptStreamEntry = {
  readonly id: number
  readonly role: string
  readonly text: string
  readonly turnId?: number | null
  readonly mergeKey?: string | null
  readonly sourceText?: string | null
  readonly promptId?: string | null
  readonly sourceAttachmentId?: string | null
}

export type TranscriptStreamMetadata = {
  readonly promptId?: string | null
  readonly sourceAttachmentId?: string | null
}

export type TranscriptProviderChunkOptions = {
  readonly role: string
  readonly chunk: string
  readonly mergeKey?: string | null
  readonly sourceText?: string | null
  readonly metadata?: TranscriptStreamMetadata
}

export type TranscriptStreamApplyResult<TEntry extends TranscriptStreamEntry> = {
  readonly kind: "noop" | "appended" | "merged"
  readonly entries: TEntry[]
  readonly updatedEntryId?: number
}

export type TranscriptToolUpdateApplyResult<TEntry extends TranscriptStreamEntry> =
  TranscriptStreamApplyResult<TEntry> & {
    readonly parsedUpdate?: ToolTranscriptUpdate
    readonly mergedUpdate?: ToolTranscriptUpdate
  }

type MutableTranscriptStreamEntry =
  Omit<TranscriptStreamEntry, "text" | "turnId" | "mergeKey" | "sourceText" | "promptId" | "sourceAttachmentId"> & {
    text: string
    turnId?: number | null
    mergeKey?: string | null
    sourceText?: string | null
    promptId?: string | null
    sourceAttachmentId?: string | null
  }

export function normalizeTranscriptProviderChunk(chunk: string): string {
  return chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
}

export function computeCurrentTranscriptTurnId<TEntry extends TranscriptStreamEntry>(
  entries: readonly TEntry[],
): number | null {
  return entries.reduce<number | null>((latest, entry) => {
    if (!entry || entry.role !== "user" || entry.turnId === undefined || entry.turnId === null) {
      return latest
    }
    return entry.turnId
  }, null)
}

export function computeNextTranscriptEntryId<TEntry extends TranscriptStreamEntry>(
  entries: readonly TEntry[],
): number {
  return entries.reduce((max, entry) => Math.max(max, entry.id), 0) + 1
}

export function applyTranscriptProviderChunk<TEntry extends TranscriptStreamEntry>(
  entries: readonly TEntry[],
  options: TranscriptProviderChunkOptions,
): TranscriptStreamApplyResult<TEntry> {
  const normalized = normalizeTranscriptProviderChunk(options.chunk)
  const normalizedSource = options.sourceText === undefined || options.sourceText === null
    ? undefined
    : normalizeTranscriptProviderChunk(options.sourceText)
  const metadata = options.metadata ?? {}
  if (!normalized) {
    return { kind: "noop", entries: cloneEntries(entries) }
  }

  const nextEntries = cloneEntries(entries) as MutableTranscriptStreamEntry[]
  const currentTurnId = computeCurrentTranscriptTurnId(nextEntries)
  const mergedEntry = mergeProviderChunk(nextEntries, {
    role: options.role,
    normalized,
    normalizedSource,
    mergeKey: options.mergeKey ?? undefined,
    metadata,
    currentTurnId,
  })

  if (mergedEntry) {
    return {
      kind: "merged",
      entries: nextEntries as TEntry[],
      updatedEntryId: mergedEntry.id,
    }
  }

  const nextEntry = createTranscriptEntry(nextEntries, {
    role: options.role,
    normalized,
    normalizedSource,
    mergeKey: options.mergeKey ?? undefined,
    metadata,
  })
  nextEntries.push(nextEntry)
  return {
    kind: "appended",
    entries: nextEntries as TEntry[],
    updatedEntryId: nextEntry.id,
  }
}

export function applyTranscriptToolUpdate<TEntry extends TranscriptStreamEntry>(
  entries: readonly TEntry[],
  chunk: string,
  toolState: Map<string, ToolTranscriptUpdate>,
  metadata: TranscriptStreamMetadata = {},
): TranscriptToolUpdateApplyResult<TEntry> {
  const normalized = normalizeTranscriptProviderChunk(chunk)
  if (!normalized) {
    return { kind: "noop", entries: cloneEntries(entries) }
  }

  const parsed = parseToolTranscriptUpdate(normalized)
  if (parsed) {
    const merged = mergeToolTranscriptUpdate(toolState.get(parsed.id) ?? null, parsed)
    toolState.set(parsed.id, merged)
    return {
      ...applyTranscriptProviderChunk(entries, {
        role: "tool",
        chunk: formatToolTranscriptUpdate(merged),
        mergeKey: parsed.id,
        sourceText: JSON.stringify(merged),
        metadata,
      }),
      parsedUpdate: parsed,
      mergedUpdate: merged,
    }
  }

  return applyTranscriptProviderChunk(entries, {
    role: "tool",
    chunk: normalized,
    sourceText: normalized,
    metadata,
  })
}

function mergeProviderChunk(
  entries: MutableTranscriptStreamEntry[],
  options: {
    role: string
    normalized: string
    normalizedSource: string | undefined
    mergeKey: string | undefined
    metadata: TranscriptStreamMetadata
    currentTurnId: number | null
  },
): MutableTranscriptStreamEntry | null {
  const { role, normalized, normalizedSource, mergeKey, metadata, currentTurnId } = options

  if (mergeKey) {
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const candidate = entries[index]
      if (
        candidate?.role !== role
        || candidate.mergeKey !== mergeKey
        || !sameStreamingTurn(candidate, currentTurnId)
      ) {
        continue
      }
      applyMergedChunk(candidate, role, normalized, normalizedSource)
      applyStreamMetadata(candidate, metadata)
      return candidate
    }
  }

  const last = [...entries].reverse().find((entry) => entry.role !== "turn_toggle")
  if (
    !mergeKey
    && last?.role === role
    && sameStreamingTurn(last, currentTurnId)
    && (role === "assistant" || role === "reasoning")
  ) {
    last.text += normalized
    applyStreamMetadata(last, metadata)
    return last
  }

  return null
}

function sameStreamingTurn(entry: TranscriptStreamEntry, currentTurnId: number | null): boolean {
  return currentTurnId === null || entry.turnId === currentTurnId
}

function applyMergedChunk(
  candidate: MutableTranscriptStreamEntry,
  role: string,
  normalized: string,
  normalizedSource: string | undefined,
): void {
  if (role === "assistant" || role === "reasoning") {
    candidate.text += normalized
    if (normalizedSource !== undefined) {
      candidate.sourceText = `${candidate.sourceText ?? ""}${normalizedSource}`
    }
    return
  }

  candidate.text = normalized
  if (normalizedSource !== undefined) {
    candidate.sourceText = normalizedSource
  }
}

function createTranscriptEntry(
  entries: readonly TranscriptStreamEntry[],
  options: {
    role: string
    normalized: string
    normalizedSource: string | undefined
    mergeKey: string | undefined
    metadata: TranscriptStreamMetadata
  },
): MutableTranscriptStreamEntry {
  const nextEntry: MutableTranscriptStreamEntry = {
    id: computeNextTranscriptEntryId(entries),
    role: options.role,
    text: options.normalized,
  }
  const currentTurnId = computeCurrentTranscriptTurnId(entries)
  if (currentTurnId !== null) {
    nextEntry.turnId = currentTurnId
  }
  if (options.mergeKey) {
    nextEntry.mergeKey = options.mergeKey
  }
  if (options.normalizedSource !== undefined) {
    nextEntry.sourceText = options.normalizedSource
  }
  applyStreamMetadata(nextEntry, options.metadata)
  return nextEntry
}

function applyStreamMetadata(entry: MutableTranscriptStreamEntry, metadata: TranscriptStreamMetadata): void {
  if (metadata.promptId !== undefined) {
    entry.promptId = metadata.promptId
  }
  if (metadata.sourceAttachmentId !== undefined) {
    entry.sourceAttachmentId = metadata.sourceAttachmentId
  }
}

function cloneEntries<TEntry extends TranscriptStreamEntry>(entries: readonly TEntry[]): TEntry[] {
  return entries.map((entry) => ({ ...entry }))
}
