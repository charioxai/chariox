import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
} from "@arroba/tool-display"
import {
  type ExternalProviderObservedMutableTranscriptMetadataFields,
} from "./external-provider-observation.js"
import {
  mergeSessionHistoryPromptAttachments,
} from "./session-history-attachments.js"
import {
  applyTranscriptHistoryDeferral,
  markDeferredTranscriptHistoryEntries,
  transcriptHistoryFragmentsAreAdjacent,
} from "./session-history-fragments.js"
import type {
  SessionHistoryPromptAttachment,
  TranscriptEntry as KernelTranscriptEntry,
} from "./kernel-types.js"

export type TranscriptHistoryStitchEntry = ExternalProviderObservedMutableTranscriptMetadataFields & {
  id: KernelTranscriptEntry["id"]
  role: string
  text: KernelTranscriptEntry["text"]
  sourceText?: KernelTranscriptEntry["sourceText"]
  mergeKey?: KernelTranscriptEntry["mergeKey"]
  promptId?: string | null
  promptOrigin?: string | null
  sourceAttachmentId?: string | null
  attachments?: SessionHistoryPromptAttachment[]
  providerRunId?: string | null
  historyTurnCompletedAtMs?: number | null
  historyTurnLifecycle?: "open" | "completed" | "cancelled"
  historyDeferred?: boolean
  historyEntryIndex?: number
  historyFragmentStart?: number
  historyFragmentEnd?: number
  historyTotalChars?: number
}

export function mergePrependedTranscriptHistoryFragments<TEntry extends TranscriptHistoryStitchEntry>(
  older: TEntry,
  newer: TEntry,
): TEntry {
  const sourceText = (older.sourceText ?? older.text) + (newer.sourceText ?? newer.text)
  const mergedBase = {
    ...newer,
    text: newer.text,
    sourceText,
  } as TEntry
  if (older.historyFragmentStart !== undefined) mergedBase.historyFragmentStart = older.historyFragmentStart
  if (newer.historyFragmentEnd !== undefined) mergedBase.historyFragmentEnd = newer.historyFragmentEnd
  const totalChars = newer.historyTotalChars ?? older.historyTotalChars
  if (totalChars !== undefined) mergedBase.historyTotalChars = totalChars
  if (older.attachments !== undefined || newer.attachments !== undefined) {
    mergedBase.attachments = mergeSessionHistoryPromptAttachments(
      older.attachments,
      newer.attachments,
    )
  }
  if (older.role !== "tool") {
    return applyTranscriptHistoryDeferral({
      ...mergedBase,
      text: older.text + newer.text,
    })
  }

  const parsed = parseToolTranscriptUpdate(sourceText)
  if (!parsed) {
    const pending = {
      ...mergedBase,
      text: sourceText,
    }
    delete pending.mergeKey
    return applyTranscriptHistoryDeferral(pending)
  }

  const merged = mergeToolTranscriptUpdate(null, parsed)
  return applyTranscriptHistoryDeferral({
    ...mergedBase,
    text: formatToolTranscriptUpdate(merged),
    mergeKey: parsed.id,
  })
}

export function stitchPrependedTranscriptHistory<TEntry extends TranscriptHistoryStitchEntry>(
  olderEntries: readonly TEntry[],
  currentEntries: readonly TEntry[],
): TEntry[] {
  if (olderEntries.length === 0 || currentEntries.length === 0) {
    return markDeferredTranscriptHistoryEntries([...olderEntries, ...currentEntries])
  }

  const tail = olderEntries.at(-1)
  const head = currentEntries[0]
  if (!tail || !head) {
    return markDeferredTranscriptHistoryEntries([...olderEntries, ...currentEntries])
  }
  if (!transcriptHistoryFragmentsAreAdjacent(tail, head)) {
    return markDeferredTranscriptHistoryEntries([...olderEntries, ...currentEntries])
  }
  if (
    tail.role !== head.role
    || !transcriptHistoryFragmentIdentityMetadataMatches(tail, head)
  ) {
    return markDeferredTranscriptHistoryEntries([...olderEntries, ...currentEntries])
  }

  return markDeferredTranscriptHistoryEntries([
    ...olderEntries.slice(0, -1),
    mergePrependedTranscriptHistoryFragments(tail, head),
    ...currentEntries.slice(1),
  ])
}

function transcriptHistoryFragmentIdentityMetadataMatches(
  older: TranscriptHistoryStitchEntry,
  newer: TranscriptHistoryStitchEntry,
): boolean {
  return nullableField(older.providerRunId) === nullableField(newer.providerRunId)
    && nullableField(older.promptId) === nullableField(newer.promptId)
    && nullableField(older.promptOrigin) === nullableField(newer.promptOrigin)
    && nullableField(older.sourceAttachmentId) === nullableField(newer.sourceAttachmentId)
    && nullableField(older.historyTurnCompletedAtMs) === nullableField(newer.historyTurnCompletedAtMs)
    && nullableField(older.historyTurnLifecycle) === nullableField(newer.historyTurnLifecycle)
    && nullableField(older.source) === nullableField(newer.source)
    && nullableField(older.externalProvider) === nullableField(newer.externalProvider)
    && nullableField(older.externalProviderSessionId) === nullableField(newer.externalProviderSessionId)
    && nullableField(older.externalProviderTurnId) === nullableField(newer.externalProviderTurnId)
    && nullableField(older.observedAtMs) === nullableField(newer.observedAtMs)
    && normalizedExternalObservation(older) === normalizedExternalObservation(newer)
}

function nullableField(value: string | number | null | undefined): string | number | undefined {
  return value ?? undefined
}

function normalizedExternalObservation(entry: TranscriptHistoryStitchEntry): string | undefined {
  return entry.externalObservation === undefined || entry.externalObservation === null
    ? undefined
    : JSON.stringify(entry.externalObservation)
}
