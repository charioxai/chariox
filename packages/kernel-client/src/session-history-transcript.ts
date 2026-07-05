import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"
import {
  externalProviderObservedProviderStatusShouldRender,
  historyEntryExternalProviderObservedMetadata,
  mergeExternalProviderObservation,
} from "./external-provider-observation.js"
import { previewLineForSessionHistoryEntry } from "./session-history-preview.js"
import {
  cloneSessionHistoryPromptAttachments,
  mergeSessionHistoryPromptAttachments,
} from "./session-history-attachments.js"
import {
  applyTranscriptHistoryDeferral,
  markDeferredTranscriptHistoryEntries,
} from "./session-history-fragments.js"
import {
  mergePrependedTranscriptHistoryFragments,
  stitchPrependedTranscriptHistory,
} from "./transcript-history-stitching.js"
import { trimSingleTrailingNewline } from "./transcript-entry-state.js"
import { sessionHistoryEntryKindTranscriptRole } from "./session-history-outline.js"
import { mergeAdjacentSessionHistoryPageEntries } from "./session-history-page-entries.js"
import type {
  SessionHistoryEntry,
  SessionHistoryExternalObservation,
  SessionHistoryPageEntry,
  SessionHistoryPromptAttachment,
} from "./kernel-types.js"

export type SessionHistoryTranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "error" | "status" | "notice" | "turn_toggle"
  text: string
  promptId?: string | null
  sourceAttachmentId?: string | null
  attachments?: SessionHistoryPromptAttachment[]
  sourceText?: string
  mergeKey?: string
  providerRunId?: string | null
  source?: "external_provider_observed" | string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
  externalObservation?: SessionHistoryExternalObservation | null
  emphasis?: "muted" | "warning" | "error"
  turnId?: number
  historyDeferred?: boolean
  historyEntryIndex?: number
  historyFragmentStart?: number
  historyFragmentEnd?: number
  historyTotalChars?: number
}

export type SessionHistoryTranscriptHydrateOptions = {
  promptId?: string | null
}

export function applyHistoryTranscriptDeferral<TEntry extends SessionHistoryTranscriptEntry>(entry: TEntry): TEntry {
  return applyTranscriptHistoryDeferral(entry)
}

export function markDeferredHistoryTranscriptEntries<TEntry extends SessionHistoryTranscriptEntry>(
  items: TEntry[],
): TEntry[] {
  return markDeferredTranscriptHistoryEntries(items)
}

export function mergePrependedHistoryTranscriptFragments<TEntry extends SessionHistoryTranscriptEntry>(
  older: TEntry,
  newer: TEntry,
): TEntry {
  return mergePrependedTranscriptHistoryFragments(older, newer) as TEntry
}

export function stitchPrependedHistoryTranscript<TEntry extends SessionHistoryTranscriptEntry>(
  olderEntries: TEntry[],
  currentEntries: TEntry[],
): TEntry[] {
  return stitchPrependedTranscriptHistory(olderEntries, currentEntries) as TEntry[]
}

export function hydrateSessionHistoryTranscriptEntries(
  historyEntries: SessionHistoryPageEntry[],
  hydrateOptions: SessionHistoryTranscriptHydrateOptions = {},
): SessionHistoryTranscriptEntry[] {
  const mergedHistoryEntries = mergeAdjacentSessionHistoryPageEntries(historyEntries)
  const entries: SessionHistoryTranscriptEntry[] = []
  const tools = new Map<string, ToolTranscriptUpdate>()
  let nextId = 0
  let currentTurnId = 0

  const appendTranscriptEntry = (
    role: SessionHistoryTranscriptEntry["role"],
    chunk: string,
    options: SessionHistoryTranscriptMetadataOptions & {
      mergeKey?: string
      sourceText?: string
      emphasis?: SessionHistoryTranscriptEntry["emphasis"]
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
          && sameSessionHistoryTranscriptIdentity(candidate, options)
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
          applySessionHistoryTranscriptMetadata(candidate, options)
          return
        }
      }
    }

    const last = entries.at(-1)
    if (!options.mergeKey && last?.role === role && role === "reasoning") {
      last.text += normalized
      applySessionHistoryTranscriptMetadata(last, options)
      return
    }

    nextId += 1
    const nextEntry: SessionHistoryTranscriptEntry = { id: nextId, role, text: normalized }
    if (options.mergeKey) nextEntry.mergeKey = options.mergeKey
    if (options.sourceText !== undefined) nextEntry.sourceText = options.sourceText
    if (options.emphasis !== undefined) nextEntry.emphasis = options.emphasis
    if (options.turnId !== undefined) nextEntry.turnId = options.turnId
    applySessionHistoryTranscriptMetadata(nextEntry, options)
    entries.push(nextEntry)
  }

  for (const pageEntry of mergedHistoryEntries) {
    const entryOptions: SessionHistoryTranscriptMetadataOptions & { turnId?: number } = {
      historyEntryIndex: pageEntry.entry_index,
      historyFragmentStart: pageEntry.fragment_start,
      historyFragmentEnd: pageEntry.fragment_end,
      historyTotalChars: pageEntry.total_chars,
    }
    if (pageEntry.entry.provider_run_id !== undefined) entryOptions.providerRunId = pageEntry.entry.provider_run_id
    if (currentTurnId > 0) entryOptions.turnId = currentTurnId
    const observedOptions = historyEntryExternalProviderObservedMetadata(pageEntry.entry) ?? {}
    const identityOptions = historyEntryTranscriptIdentityOptions(pageEntry.entry, hydrateOptions.promptId)
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

  return markDeferredHistoryTranscriptEntries(entries)
}

export function previewLineForHistoryTranscriptEntry(entry: SessionHistoryEntry): string | null {
  return previewLineForSessionHistoryEntry(entry)
}

function applySessionHistoryTranscriptMetadata(
  entry: SessionHistoryTranscriptEntry,
  options: SessionHistoryTranscriptMetadataOptions,
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
  if (options.attachments !== undefined) {
    entry.attachments = mergeSessionHistoryPromptAttachments(entry.attachments, options.attachments)
  }
  if (options.historyEntryIndex !== undefined) entry.historyEntryIndex = options.historyEntryIndex
  if (options.historyFragmentStart !== undefined) entry.historyFragmentStart = options.historyFragmentStart
  if (options.historyFragmentEnd !== undefined) entry.historyFragmentEnd = options.historyFragmentEnd
  if (options.historyTotalChars !== undefined) entry.historyTotalChars = options.historyTotalChars
}

type SessionHistoryTranscriptMetadataOptions = {
  providerRunId?: string | null | undefined
  source?: SessionHistoryTranscriptEntry["source"] | undefined
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
  observedAtMs?: number | null | undefined
  externalObservation?: SessionHistoryTranscriptEntry["externalObservation"] | undefined
  promptId?: string | null | undefined
  sourceAttachmentId?: string | null | undefined
  attachments?: SessionHistoryTranscriptEntry["attachments"] | undefined
  historyEntryIndex?: number | undefined
  historyFragmentStart?: number | undefined
  historyFragmentEnd?: number | undefined
  historyTotalChars?: number | undefined
}

function sameSessionHistoryTranscriptIdentity(
  candidate: SessionHistoryTranscriptEntry,
  options: Pick<SessionHistoryTranscriptMetadataOptions, "providerRunId"> & { turnId?: number },
) {
  if (options.providerRunId !== undefined && candidate.providerRunId !== options.providerRunId) {
    return false
  }
  if (options.turnId !== undefined && candidate.turnId !== options.turnId) {
    return false
  }
  return true
}

function historyEntryTranscriptIdentityOptions(
  entry: SessionHistoryEntry,
  turnPromptId?: string | null,
): Partial<SessionHistoryTranscriptEntry> {
  return {
    ...(turnPromptId !== undefined ? { promptId: turnPromptId } : {}),
    ...(entry.source_attachment_id !== undefined ? { sourceAttachmentId: entry.source_attachment_id } : {}),
    ...(entry.attachments !== undefined ? { attachments: cloneSessionHistoryPromptAttachments(entry.attachments) } : {}),
  }
}
