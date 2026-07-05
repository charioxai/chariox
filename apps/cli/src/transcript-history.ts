import {
  externalProviderObservedProviderStatusShouldRender,
  historyEntryExternalProviderObservedMetadata,
  mergeExternalProviderObservation,
  mergeExternalProviderObservedTranscriptFields,
} from "@arroba/kernel-client/external-provider-observation"
import { previewLineForSessionHistoryEntry } from "@arroba/kernel-client/session-history-preview"
import {
  cloneSessionHistoryPromptAttachments,
  mergeSessionHistoryPromptAttachments,
} from "@arroba/kernel-client/session-history-attachments"
import {
  applyTranscriptHistoryDeferral,
  markDeferredTranscriptHistoryEntries,
  transcriptHistoryFragmentsAreAdjacent,
} from "@arroba/kernel-client/session-history-fragments"
import { sessionHistoryEntryKindTranscriptRole } from "@arroba/kernel-client/session-history-outline"
import { mergeAdjacentSessionHistoryPageEntries } from "@arroba/kernel-client/session-history-page-entries"
import type { SessionHistoryEntry, SessionHistoryPageEntry, TranscriptEntry } from "./cli-types.js"
import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import { trimSingleTrailingNewline } from "./transcript-text.js"

export function applyHistoryDeferral(entry: TranscriptEntry) {
  return applyTranscriptHistoryDeferral(entry)
}

export function markDeferredHistoryEntries(items: TranscriptEntry[]) {
  return markDeferredTranscriptHistoryEntries(items)
}

export function mergePrependedHistoryFragments(older: TranscriptEntry, newer: TranscriptEntry): TranscriptEntry {
  const sourceText = (older.sourceText ?? older.text) + (newer.sourceText ?? newer.text)
  const mergedBase: TranscriptEntry = {
    ...newer,
    text: newer.text,
    sourceText,
  }
  mergeStitchedHistoryMetadata(mergedBase, older, newer)
  if (older.historyFragmentStart !== undefined) mergedBase.historyFragmentStart = older.historyFragmentStart
  if (newer.historyFragmentEnd !== undefined) mergedBase.historyFragmentEnd = newer.historyFragmentEnd
  const totalChars = newer.historyTotalChars ?? older.historyTotalChars
  if (totalChars !== undefined) mergedBase.historyTotalChars = totalChars
  if (older.role !== "tool") {
    return applyHistoryDeferral({
      ...mergedBase,
      text: older.text + newer.text,
    })
  }

  const parsed = parseToolTranscriptUpdate(sourceText)
  if (!parsed) {
    const pending: TranscriptEntry = {
      ...mergedBase,
      text: sourceText,
    }
    delete pending.mergeKey
    return {
      ...applyHistoryDeferral(pending),
    }
  }

  const merged = mergeToolTranscriptUpdate(null, parsed)
  return applyHistoryDeferral({
    ...mergedBase,
    text: formatToolTranscriptUpdate(merged),
    mergeKey: parsed.id,
  })
}

export function stitchPrependedHistory(olderEntries: TranscriptEntry[], currentEntries: TranscriptEntry[]) {
  if (olderEntries.length === 0 || currentEntries.length === 0) {
    return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
  }

  const tail = olderEntries.at(-1)
  const head = currentEntries[0]
  if (!tail || !head) {
    return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
  }
  if (!transcriptHistoryFragmentsAreAdjacent(tail, head)) {
    return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
  }

  return markDeferredHistoryEntries([
    ...olderEntries.slice(0, -1),
    mergePrependedHistoryFragments(tail, head),
    ...currentEntries.slice(1),
  ])
}

export function mergeAdjacentHistoryPageEntries(historyEntries: SessionHistoryPageEntry[]) {
  return mergeAdjacentSessionHistoryPageEntries(historyEntries)
}

export function hydrateTranscriptEntries(
  historyEntries: SessionHistoryPageEntry[],
  hydrateOptions: { promptId?: string | null } = {},
): TranscriptEntry[] {
  const mergedHistoryEntries = mergeAdjacentHistoryPageEntries(historyEntries)
  const entries: TranscriptEntry[] = []
  const tools = new Map<string, ToolTranscriptUpdate>()
  let nextId = 0
  let currentTurnId = 0

  const appendTranscriptEntry = (
    role: TranscriptEntry["role"],
    chunk: string,
    options: {
      mergeKey?: string
      providerRunId?: string | null
      sourceText?: string
      source?: TranscriptEntry["source"]
      externalProvider?: string | null
      externalProviderSessionId?: string | null
      externalProviderTurnId?: string | null
      observedAtMs?: number | null
      externalObservation?: TranscriptEntry["externalObservation"]
      promptId?: string | null
      sourceAttachmentId?: string | null
      attachments?: TranscriptEntry["attachments"]
      emphasis?: TranscriptEntry["emphasis"]
      historyEntryIndex?: number
      historyFragmentStart?: number
      historyFragmentEnd?: number
      historyTotalChars?: number
      turnId?: number
    } = {},
  ) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }

    if (options.mergeKey) {
      for (let index = entries.length - 1; index >= 0; index -= 1) {
        const candidate = entries[index]
        if (
          candidate?.role === role
          && candidate.mergeKey === options.mergeKey
          && sameTranscriptHistoryIdentity(candidate, options)
        ) {
          if (role === "assistant" || role === "reasoning") {
            candidate.text += normalized
            if (options.sourceText !== undefined) {
              candidate.sourceText = `${candidate.sourceText ?? ""}${options.sourceText}`
            }
          } else {
            candidate.text = normalized
            if (options.sourceText !== undefined) candidate.sourceText = options.sourceText
          }
          if (options.emphasis !== undefined) candidate.emphasis = options.emphasis
          if (options.providerRunId !== undefined) candidate.providerRunId = options.providerRunId
          if (options.source !== undefined) candidate.source = options.source
          if (options.externalProvider !== undefined) candidate.externalProvider = options.externalProvider
          if (options.externalProviderSessionId !== undefined) candidate.externalProviderSessionId = options.externalProviderSessionId
          if (options.externalProviderTurnId !== undefined) candidate.externalProviderTurnId = options.externalProviderTurnId
          if (options.observedAtMs !== undefined) candidate.observedAtMs = options.observedAtMs
          if (options.externalObservation !== undefined) {
            const externalObservation = mergeExternalProviderObservation(
              candidate.externalObservation,
              options.externalObservation,
            )
            if (externalObservation !== undefined) candidate.externalObservation = externalObservation
          }
          if (options.promptId !== undefined) candidate.promptId = options.promptId
          if (options.sourceAttachmentId !== undefined) candidate.sourceAttachmentId = options.sourceAttachmentId
          if (options.attachments !== undefined) candidate.attachments = mergeSessionHistoryPromptAttachments(candidate.attachments, options.attachments)
          if (options.historyEntryIndex !== undefined) candidate.historyEntryIndex = options.historyEntryIndex
          if (options.historyFragmentStart !== undefined) candidate.historyFragmentStart = options.historyFragmentStart
          if (options.historyFragmentEnd !== undefined) candidate.historyFragmentEnd = options.historyFragmentEnd
          if (options.historyTotalChars !== undefined) candidate.historyTotalChars = options.historyTotalChars
          return
        }
      }
    }

    const last = entries.at(-1)
    if (!options.mergeKey && last?.role === role && role === "reasoning") {
      last.text += normalized
      applyEntryMetadata(last, options)
      return
    }

    nextId += 1
    const nextEntry: TranscriptEntry = { id: nextId, role, text: normalized }
    if (options.mergeKey) {
      nextEntry.mergeKey = options.mergeKey
    }
    if (options.providerRunId !== undefined) nextEntry.providerRunId = options.providerRunId
    if (options.sourceText !== undefined) nextEntry.sourceText = options.sourceText
    if (options.source !== undefined) nextEntry.source = options.source
    if (options.externalProvider !== undefined) nextEntry.externalProvider = options.externalProvider
    if (options.externalProviderSessionId !== undefined) nextEntry.externalProviderSessionId = options.externalProviderSessionId
    if (options.externalProviderTurnId !== undefined) nextEntry.externalProviderTurnId = options.externalProviderTurnId
    if (options.observedAtMs !== undefined) nextEntry.observedAtMs = options.observedAtMs
    if (options.externalObservation !== undefined) {
      const externalObservation = mergeExternalProviderObservation(
        nextEntry.externalObservation,
        options.externalObservation,
      )
      if (externalObservation !== undefined) nextEntry.externalObservation = externalObservation
    }
    if (options.promptId !== undefined) nextEntry.promptId = options.promptId
    if (options.sourceAttachmentId !== undefined) nextEntry.sourceAttachmentId = options.sourceAttachmentId
    if (options.attachments !== undefined) nextEntry.attachments = cloneSessionHistoryPromptAttachments(options.attachments)
    if (options.emphasis !== undefined) nextEntry.emphasis = options.emphasis
    if (options.historyEntryIndex !== undefined) nextEntry.historyEntryIndex = options.historyEntryIndex
    if (options.historyFragmentStart !== undefined) nextEntry.historyFragmentStart = options.historyFragmentStart
    if (options.historyFragmentEnd !== undefined) nextEntry.historyFragmentEnd = options.historyFragmentEnd
    if (options.historyTotalChars !== undefined) nextEntry.historyTotalChars = options.historyTotalChars
    if (options.turnId !== undefined) nextEntry.turnId = options.turnId
    entries.push(nextEntry)
  }

  for (const pageEntry of mergedHistoryEntries) {
    const entryOptions: {
      historyEntryIndex: number
      historyFragmentStart: number
      historyFragmentEnd: number
      historyTotalChars: number
      providerRunId?: string | null
      turnId?: number
    } = {
      historyEntryIndex: pageEntry.entry_index,
      historyFragmentStart: pageEntry.fragment_start,
      historyFragmentEnd: pageEntry.fragment_end,
      historyTotalChars: pageEntry.total_chars,
    }
    if (pageEntry.entry.provider_run_id !== undefined) entryOptions.providerRunId = pageEntry.entry.provider_run_id
    if (currentTurnId > 0) entryOptions.turnId = currentTurnId
    const observedOptions = externalProviderObservedOptions(pageEntry.entry)
    const identityOptions = historyEntryIdentityOptions(pageEntry.entry, hydrateOptions.promptId)
    const role = sessionHistoryEntryKindTranscriptRole(pageEntry.entry.kind)
    switch (role) {
      case "user":
        currentTurnId = Math.max(currentTurnId + 1, (pageEntry.entry_index ?? 0) + 1)
        appendTranscriptEntry("user", trimSingleTrailingNewline(pageEntry.entry.text), {
          ...entryOptions,
          ...observedOptions,
          ...identityOptions,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
          turnId: currentTurnId,
        })
        break
      case "reasoning":
        appendTranscriptEntry("reasoning", pageEntry.entry.text, {
          ...entryOptions,
          ...observedOptions,
          ...identityOptions,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
        })
        break
      case "tool": {
        const parsed = parseToolTranscriptUpdate(pageEntry.entry.text)
        if (!parsed) {
          appendTranscriptEntry("tool", pageEntry.entry.text, {
            ...entryOptions,
            sourceText: pageEntry.entry.text,
            ...observedOptions,
            ...identityOptions,
          })
          break
        }
        const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
        tools.set(parsed.id, merged)
        appendTranscriptEntry("tool", formatToolTranscriptUpdate(merged), {
          ...entryOptions,
          mergeKey: parsed.id,
          sourceText: pageEntry.entry.text,
          ...observedOptions,
          ...identityOptions,
        })
        break
      }
      case "error":
        appendTranscriptEntry("error", pageEntry.entry.text, {
          ...entryOptions,
          ...observedOptions,
          ...identityOptions,
          emphasis: "error",
        })
        break
      case "status":
        if (externalProviderObservedProviderStatusShouldRender(pageEntry.entry)) {
          appendTranscriptEntry("status", pageEntry.entry.text, {
            ...entryOptions,
            ...observedOptions,
            ...identityOptions,
            mergeKey: "__provider_status__",
          })
        }
        break
      case "notice":
        appendTranscriptEntry("notice", pageEntry.entry.text, {
          ...entryOptions,
          ...observedOptions,
          ...identityOptions,
        })
        break
      case "assistant":
        appendTranscriptEntry("assistant", pageEntry.entry.text, {
          ...entryOptions,
          ...observedOptions,
          ...identityOptions,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
        })
        break
    }
  }

  return markDeferredHistoryEntries(entries)
}

function applyEntryMetadata(
  entry: TranscriptEntry,
  options: TranscriptEntryMetadataOptions,
) {
  if (options.providerRunId !== undefined) entry.providerRunId = options.providerRunId
  if (options.source !== undefined) entry.source = options.source
  if (options.externalProvider !== undefined) entry.externalProvider = options.externalProvider
  if (options.externalProviderSessionId !== undefined) entry.externalProviderSessionId = options.externalProviderSessionId
  if (options.externalProviderTurnId !== undefined) entry.externalProviderTurnId = options.externalProviderTurnId
  if (options.observedAtMs !== undefined) entry.observedAtMs = options.observedAtMs
  if (options.externalObservation !== undefined) {
    const externalObservation = mergeExternalProviderObservation(
      entry.externalObservation,
      options.externalObservation,
    )
    if (externalObservation !== undefined) entry.externalObservation = externalObservation
  }
  if (options.promptId !== undefined) entry.promptId = options.promptId
  if (options.sourceAttachmentId !== undefined) entry.sourceAttachmentId = options.sourceAttachmentId
  if (options.attachments !== undefined) entry.attachments = mergeSessionHistoryPromptAttachments(entry.attachments, options.attachments)
  if (options.historyEntryIndex !== undefined) entry.historyEntryIndex = options.historyEntryIndex
  if (options.historyFragmentStart !== undefined) entry.historyFragmentStart = options.historyFragmentStart
  if (options.historyFragmentEnd !== undefined) entry.historyFragmentEnd = options.historyFragmentEnd
  if (options.historyTotalChars !== undefined) entry.historyTotalChars = options.historyTotalChars
}

type TranscriptEntryMetadataOptions = {
  providerRunId?: string | null | undefined
  source?: TranscriptEntry["source"] | undefined
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
  observedAtMs?: number | null | undefined
  externalObservation?: TranscriptEntry["externalObservation"] | undefined
  promptId?: string | null | undefined
  sourceAttachmentId?: string | null | undefined
  attachments?: TranscriptEntry["attachments"] | undefined
  historyEntryIndex?: number | undefined
  historyFragmentStart?: number | undefined
  historyFragmentEnd?: number | undefined
  historyTotalChars?: number | undefined
}

function sameTranscriptHistoryIdentity(
  candidate: TranscriptEntry,
  options: Pick<TranscriptEntryMetadataOptions, "providerRunId"> & { turnId?: number },
) {
  if (options.providerRunId !== undefined && candidate.providerRunId !== options.providerRunId) {
    return false
  }
  if (options.turnId !== undefined && candidate.turnId !== options.turnId) {
    return false
  }
  return true
}

function mergeStitchedHistoryMetadata(
  target: TranscriptEntry,
  older: TranscriptEntry,
  newer: TranscriptEntry,
) {
  if (target.providerRunId === undefined && older.providerRunId !== undefined) {
    target.providerRunId = older.providerRunId
  }
  if (target.source === undefined && older.source !== undefined) {
    target.source = older.source
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

function externalProviderObservedOptions(entry: SessionHistoryEntry): Partial<TranscriptEntry> {
  return historyEntryExternalProviderObservedMetadata(entry) ?? {}
}

function historyEntryIdentityOptions(
  entry: SessionHistoryEntry,
  turnPromptId?: string | null,
): Partial<TranscriptEntry> {
  return {
    ...(turnPromptId !== undefined ? { promptId: turnPromptId } : {}),
    ...(entry.source_attachment_id !== undefined ? { sourceAttachmentId: entry.source_attachment_id } : {}),
    ...(entry.attachments !== undefined ? { attachments: cloneSessionHistoryPromptAttachments(entry.attachments) } : {}),
  }
}

export function previewLineForHistoryEntry(entry: SessionHistoryEntry) {
  return previewLineForSessionHistoryEntry(entry)
}
