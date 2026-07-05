import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"
import {
  computeCurrentTranscriptTurnId,
  computeNextTranscriptEntryId,
} from "./transcript-entry-state.js"
import type { TranscriptEntry as KernelTranscriptEntry } from "./kernel-types.js"

export {
  computeCurrentTranscriptTurnId,
  computeNextTranscriptEntryId,
} from "./transcript-entry-state.js"

type TranscriptStreamKernelFields = Pick<KernelTranscriptEntry, "id" | "text"> & {
  readonly turnId?: KernelTranscriptEntry["turnId"] | null
  readonly mergeKey?: KernelTranscriptEntry["mergeKey"] | null
  readonly sourceText?: KernelTranscriptEntry["sourceText"] | null
}

export type TranscriptStreamEntry = {
  readonly id: TranscriptStreamKernelFields["id"]
  readonly role: string
  readonly text: TranscriptStreamKernelFields["text"]
  readonly turnId?: TranscriptStreamKernelFields["turnId"]
  readonly providerRunId?: string | null
  readonly mergeKey?: TranscriptStreamKernelFields["mergeKey"]
  readonly sourceText?: TranscriptStreamKernelFields["sourceText"]
  readonly promptId?: string | null
  readonly sourceAttachmentId?: string | null
}

export type TranscriptStreamMetadata = {
  readonly promptId?: string | null | undefined
  readonly sourceAttachmentId?: string | null | undefined
}

export type TranscriptProviderChunkOptions = {
  readonly role: string
  readonly chunk: string
  readonly mergeKey?: string | null | undefined
  readonly sourceText?: string | null | undefined
  readonly metadata?: TranscriptStreamMetadata
  readonly nextEntryId?: number | null | undefined
  readonly currentTurnId?: number | null | undefined
  readonly providerRunId?: string | null | undefined
  readonly mergeAdjacentUnkeyedRoles?: readonly string[] | undefined
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
  Omit<TranscriptStreamEntry, "text" | "turnId" | "providerRunId" | "mergeKey" | "sourceText" | "promptId" | "sourceAttachmentId"> & {
    text: string
    turnId?: number | null
    providerRunId?: string | null
    mergeKey?: string | null
    sourceText?: string | null
    promptId?: string | null
    sourceAttachmentId?: string | null
  }

export function normalizeTranscriptProviderChunk(chunk: string): string {
  return chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
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
  const currentTurnId = options.currentTurnId !== undefined
    ? options.currentTurnId
    : computeCurrentTranscriptTurnId(nextEntries)
  const mergedEntry = mergeProviderChunk(nextEntries, {
    role: options.role,
    normalized,
    normalizedSource,
    mergeKey: options.mergeKey ?? undefined,
    metadata,
    currentTurnId,
    providerRunId: options.providerRunId,
    mergeAdjacentUnkeyedRoles: options.mergeAdjacentUnkeyedRoles ?? ["assistant", "reasoning"],
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
    nextEntryId: options.nextEntryId ?? undefined,
    currentTurnId,
    providerRunId: options.providerRunId,
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
  options: {
    readonly nextEntryId?: number | null
    readonly currentTurnId?: number | null
    readonly providerRunId?: string | null
    readonly mergeAdjacentUnkeyedRoles?: readonly string[]
  } = {},
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
        nextEntryId: options.nextEntryId ?? undefined,
        currentTurnId: options.currentTurnId,
        providerRunId: options.providerRunId,
        mergeAdjacentUnkeyedRoles: options.mergeAdjacentUnkeyedRoles,
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
    nextEntryId: options.nextEntryId ?? undefined,
    currentTurnId: options.currentTurnId,
    providerRunId: options.providerRunId,
    mergeAdjacentUnkeyedRoles: options.mergeAdjacentUnkeyedRoles,
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
    providerRunId: string | null | undefined
    mergeAdjacentUnkeyedRoles: readonly string[]
  },
): MutableTranscriptStreamEntry | null {
  const {
    role,
    normalized,
    normalizedSource,
    mergeKey,
    metadata,
    currentTurnId,
    providerRunId,
    mergeAdjacentUnkeyedRoles,
  } = options

  if (mergeKey) {
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const candidate = entries[index]
      if (
        candidate?.role !== role
        || candidate.mergeKey !== mergeKey
        || !sameStreamingMergeIdentity(candidate, { currentTurnId, providerRunId })
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
    && sameStreamingMergeIdentity(last, { currentTurnId, providerRunId })
    && mergeAdjacentUnkeyedRoles.includes(role)
  ) {
    last.text += normalized
    applyStreamMetadata(last, metadata)
    return last
  }

  return null
}

function sameStreamingMergeIdentity(
  entry: TranscriptStreamEntry,
  options: { currentTurnId: number | null; providerRunId: string | null | undefined },
): boolean {
  if (options.currentTurnId !== null && entry.turnId !== options.currentTurnId) {
    return false
  }
  if (options.providerRunId !== undefined && entry.providerRunId !== options.providerRunId) {
    return false
  }
  return true
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
    nextEntryId: number | undefined
    currentTurnId: number | null
    providerRunId: string | null | undefined
  },
): MutableTranscriptStreamEntry {
  const nextEntry: MutableTranscriptStreamEntry = {
    id: options.nextEntryId ?? computeNextTranscriptEntryId(entries),
    role: options.role,
    text: options.normalized,
  }
  if (options.currentTurnId !== null) {
    nextEntry.turnId = options.currentTurnId
  }
  if (options.providerRunId !== undefined) {
    nextEntry.providerRunId = options.providerRunId
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
