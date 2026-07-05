import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
} from "@arroba/tool-display"
import {
  mergeExternalProviderObservedTranscriptFields,
  mergeExternalProviderObservedSource,
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
  SessionHistoryExternalObservation,
  SessionHistoryPromptAttachment,
} from "./kernel-types.js"

export type TranscriptHistoryStitchEntry = {
  id: number
  role: string
  text: string
  sourceText?: string
  mergeKey?: string
  source?: "external_provider_observed" | string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
  externalObservation?: SessionHistoryExternalObservation | null
  promptId?: string | null
  sourceAttachmentId?: string | null
  attachments?: SessionHistoryPromptAttachment[]
  providerRunId?: string | null
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
  mergeStitchedHistoryMetadata(mergedBase, older, newer)
  if (older.historyFragmentStart !== undefined) mergedBase.historyFragmentStart = older.historyFragmentStart
  if (newer.historyFragmentEnd !== undefined) mergedBase.historyFragmentEnd = newer.historyFragmentEnd
  const totalChars = newer.historyTotalChars ?? older.historyTotalChars
  if (totalChars !== undefined) mergedBase.historyTotalChars = totalChars
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

  return markDeferredTranscriptHistoryEntries([
    ...olderEntries.slice(0, -1),
    mergePrependedTranscriptHistoryFragments(tail, head),
    ...currentEntries.slice(1),
  ])
}

function mergeStitchedHistoryMetadata<TEntry extends TranscriptHistoryStitchEntry>(
  target: TEntry,
  older: TEntry,
  newer: TEntry,
): void {
  if (target.providerRunId === undefined && older.providerRunId !== undefined) {
    target.providerRunId = older.providerRunId
  }
  const source = mergeExternalProviderObservedSource(target.source, older.source)
  if (source !== undefined) {
    target.source = source
  }
  mergeExternalProviderObservedTranscriptFields(target, older, newer)
  if (target.promptId === undefined && older.promptId !== undefined) {
    target.promptId = older.promptId
  }
  if (target.sourceAttachmentId === undefined && older.sourceAttachmentId !== undefined) {
    target.sourceAttachmentId = older.sourceAttachmentId
  }
  if (older.attachments !== undefined || newer.attachments !== undefined) {
    target.attachments = mergeSessionHistoryPromptAttachments(
      older.attachments,
      newer.attachments,
    )
  }
}
