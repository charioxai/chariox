import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"
import {
  applyExternalProviderObservedTranscriptMetadata,
  applyExternalProviderObservedTurnMetadata,
  externalProviderObservedProviderStatusShouldRender,
  historyEntryExternalProviderObservedMetadata,
  promptOriginExternalProviderObservedMetadata,
  transcriptExternalProviderObservedTurnMetadata,
  type ExternalProviderObservedTurnMetadata,
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
import {
  applyTranscriptDisplayState,
  transcriptHistoryTurnLifecycleFromCompletedAtMs,
  type TranscriptHistoryTurnLifecycle,
} from "./transcript-display-state.js"
import {
  applyTranscriptPromptMetadata,
  reindexTranscriptEntries,
  trimSingleTrailingNewline,
} from "./transcript-entry-state.js"
import { applyTranscriptProviderChunk } from "./transcript-stream-state.js"
import { providerTranscriptRoleForKind } from "./transcript-kind-role.js"
import { sessionHistoryEntryKindTranscriptRole } from "./session-history-outline.js"
import {
  orderedSessionHistoryOutlineItems,
  orderedSessionHistoryOutlineTurns,
  sessionHistoryOutlineTurnCompletedAtMs,
  sessionHistoryOutlineTurnDisplayId,
  sessionHistoryOutlineTurnPromptMetadata,
  type SessionHistoryOutlineTurnPromptMetadata,
} from "./session-history-outline.js"
import { mergeAdjacentSessionHistoryPageEntries } from "./session-history-page-entries.js"
import type {
  SessionHistoryEntry,
  SessionHistoryExternalObservation,
  SessionHistoryBlobContent,
  SessionHistoryOutlineAgent,
  SessionHistoryOutlineBlob,
  SessionHistoryOutlineTurn,
  SessionHistoryPageEntry,
  SessionHistoryPromptAttachment,
  TranscriptEntry as KernelTranscriptEntry,
} from "./kernel-types.js"

export type SessionHistoryTranscriptEntry = KernelTranscriptEntry & {
  promptId?: string | null
  promptOrigin?: string | null
  sourceAttachmentId?: string | null
  attachments?: SessionHistoryPromptAttachment[]
  providerRunId?: string | null
  externalObservation?: SessionHistoryExternalObservation | null
  historyBlobId?: string
  historyBlobAgentId?: string
  historyBlobSourceId?: string
  historyBlobSourceAgentId?: string
  historyBlobLoaded?: boolean
  historyBlobLoading?: boolean
  historyBlobError?: string
  historyTurnCompletedAtMs?: number | null
  historyTurnLifecycle?: TranscriptHistoryTurnLifecycle
}

export type SessionHistoryTranscriptHydrateOptions = {
  promptId?: string | null
}

export type SessionHistoryBlobLoadTarget = {
  readonly agentId: string
  readonly blobId: string
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
  let entries: SessionHistoryTranscriptEntry[] = []
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
    const result = applyTranscriptProviderChunk(entries, {
      role,
      chunk,
      mergeKey: options.mergeKey,
      sourceText: options.sourceText,
      nextEntryId: nextId + 1,
      currentTurnId: options.turnId ?? null,
      providerRunId: options.providerRunId,
      mergeAdjacentUnkeyedRoles: ["reasoning"],
      metadata: {
        promptId: options.promptId,
        promptOrigin: options.promptOrigin,
        sourceAttachmentId: options.sourceAttachmentId,
      },
    })
    if (result.kind === "noop") {
      return
    }

    entries = result.entries as SessionHistoryTranscriptEntry[]
    if (result.updatedEntryId !== undefined) {
      nextId = Math.max(nextId, result.updatedEntryId)
    }
    const updatedEntry = result.updatedEntryId === undefined
      ? entries.at(-1)
      : entries.find((entry) => entry.id === result.updatedEntryId)
    if (!updatedEntry) {
      return
    }
    if (options.emphasis !== undefined) updatedEntry.emphasis = options.emphasis
    applySessionHistoryTranscriptMetadata(updatedEntry, options)
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

export function hydrateSessionHistoryOutlineAgentEntries(
  agent: SessionHistoryOutlineAgent,
): SessionHistoryTranscriptEntry[] {
  const entries: SessionHistoryTranscriptEntry[] = []
  let nextId = 0
  let activeTurnId: number | null = null

  orderedSessionHistoryOutlineTurns(agent.turns).forEach((turn, turnIndex) => {
    const turnId = sessionHistoryOutlineTurnDisplayId(turn, turnIndex)
    const completedAtMs = sessionHistoryOutlineTurnCompletedAtMs(turn)
    if (completedAtMs === null || completedAtMs === undefined) {
      activeTurnId = turnId
    }
    const externalMetadata = outlineTurnExternalMetadata(turn)
    const promptMetadata = sessionHistoryOutlineTurnPromptMetadata(turn)
    const promptEntries = hydrateSessionHistoryPageEntriesForTurn([turn.user_prompt], turnId, turn.prompt_id ?? null)
    for (const entry of promptEntries) {
      entries.push(applyOutlineTurnMetadata(
        applyOutlineTurnLifecycleMetadata({ ...entry, id: ++nextId }, completedAtMs),
        externalMetadata,
        promptMetadata,
      ))
    }
    for (const item of orderedSessionHistoryOutlineItems(turn)) {
      if (item.kind === "blob") {
        entries.push(applyOutlineTurnMetadata(
          applyOutlineTurnLifecycleMetadata(
            outlineBlobTranscriptEntry(item.blob, agent.agent_id, turnId, turn.prompt_id ?? null, ++nextId),
            completedAtMs,
          ),
          externalMetadata,
          promptMetadata,
        ))
        continue
      }
      const hydratedEntries = hydrateSessionHistoryPageEntriesForTurn([item.entry], turnId, turn.prompt_id ?? null)
      for (const entry of hydratedEntries) {
        entries.push(applyOutlineTurnMetadata(
          applyOutlineTurnLifecycleMetadata({ ...entry, id: ++nextId }, completedAtMs),
          externalMetadata,
          promptMetadata,
        ))
      }
    }
  })

  return applyTranscriptDisplayState(entries, [], activeTurnId) as SessionHistoryTranscriptEntry[]
}

export function replaceSessionHistoryBlobPlaceholder(
  entries: SessionHistoryTranscriptEntry[],
  entryId: number,
  content: SessionHistoryBlobContent,
  collapsedTurnIds: readonly number[],
): SessionHistoryTranscriptEntry[] {
  const placeholder = entries.find((entry) => entry.id === entryId)
  if (!placeholder?.historyBlobId) {
    return entries
  }
  const turnId = placeholder.turnId
  const externalMetadata = transcriptEntryExternalMetadata(placeholder)
  const promptMetadata: SessionHistoryOutlineTurnPromptMetadata = {
    ...(placeholder.promptOrigin !== undefined ? { promptOrigin: placeholder.promptOrigin } : {}),
    ...(placeholder.sourceAttachmentId !== undefined ? { sourceAttachmentId: placeholder.sourceAttachmentId } : {}),
  }
  const activeTurnId = transcriptHistoryTurnIsOpen(placeholder)
    && typeof turnId === "number"
    ? turnId
    : null
  const hydrated = hydrateSessionHistoryPageEntriesForTurn(content.entries, turnId, placeholder.promptId ?? null).map((entry) => {
    const next: SessionHistoryTranscriptEntry = {
      ...entry,
      blobCollapsed: false,
      historyBlobLoaded: true,
    }
    if (placeholder.historyBlobId) {
      next.historyBlobSourceId = placeholder.historyBlobId
    }
    if (placeholder.historyBlobAgentId) {
      next.historyBlobSourceAgentId = placeholder.historyBlobAgentId
    }
    return applyOutlineTurnMetadata(
      applyOutlineTurnLifecycleMetadata(next, placeholder.historyTurnCompletedAtMs),
      externalMetadata,
      promptMetadata,
    )
  })
  const replaced = entries.flatMap((entry) => entry.id === entryId ? hydrated : [entry])
  return applyTranscriptDisplayState(
    reindexTranscriptEntries(replaced, 0),
    collapsedTurnIds,
    activeTurnId,
  ) as SessionHistoryTranscriptEntry[]
}

export function markSessionHistoryBlobLoading(
  entries: SessionHistoryTranscriptEntry[],
  entryId: number,
  loading: boolean,
  error?: string | null,
): SessionHistoryTranscriptEntry[] {
  return entries.map((entry) => {
    if (entry.id !== entryId || !entry.historyBlobId) {
      return entry
    }
    const next: SessionHistoryTranscriptEntry = {
      ...entry,
      historyBlobLoading: loading,
    }
    if (loading) {
      next.blobSummary = "loading..."
      delete next.historyBlobError
      return next
    }
    if (error) {
      next.historyBlobError = error
      next.blobSummary = `failed: ${error}`
      return next
    }
    delete next.historyBlobError
    return next
  })
}

export function resolveSessionHistoryBlobLoadTarget(
  entry: SessionHistoryTranscriptEntry | null | undefined,
  collapsed: boolean,
): SessionHistoryBlobLoadTarget | null {
  if (
    collapsed
    || !entry?.historyBlobId
    || entry.historyBlobLoaded === true
    || entry.historyBlobLoading === true
    || !entry.historyBlobAgentId
  ) {
    return null
  }
  return {
    agentId: entry.historyBlobAgentId,
    blobId: entry.historyBlobId,
  }
}

export function previewLineForHistoryTranscriptEntry(entry: SessionHistoryEntry): string | null {
  return previewLineForSessionHistoryEntry(entry)
}

function applySessionHistoryTranscriptMetadata(
  entry: SessionHistoryTranscriptEntry,
  options: SessionHistoryTranscriptMetadataOptions,
) {
  if (options.providerRunId !== undefined) entry.providerRunId = options.providerRunId
  applyExternalProviderObservedTranscriptMetadata(entry, options)
  applyTranscriptPromptMetadata(entry, options)
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
  promptOrigin?: string | null | undefined
  sourceAttachmentId?: string | null | undefined
  attachments?: SessionHistoryTranscriptEntry["attachments"] | undefined
  historyEntryIndex?: number | undefined
  historyFragmentStart?: number | undefined
  historyFragmentEnd?: number | undefined
  historyTotalChars?: number | undefined
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

type OutlineTurnExternalMetadata = ExternalProviderObservedTurnMetadata
function outlineTurnExternalMetadata(
  turn: SessionHistoryOutlineTurn,
): OutlineTurnExternalMetadata | null {
  return promptOriginExternalProviderObservedMetadata(turn)
}

function applyOutlineTurnPromptMetadata(
  entry: SessionHistoryTranscriptEntry,
  metadata: SessionHistoryOutlineTurnPromptMetadata,
): SessionHistoryTranscriptEntry {
  return applyTranscriptPromptMetadata({ ...entry }, metadata, { preserveExisting: true })
}

function applyOutlineTurnMetadata(
  entry: SessionHistoryTranscriptEntry,
  externalMetadata: OutlineTurnExternalMetadata | null,
  promptMetadata: SessionHistoryOutlineTurnPromptMetadata,
): SessionHistoryTranscriptEntry {
  return applyOutlineTurnPromptMetadata(
    applyOutlineTurnExternalMetadata(entry, externalMetadata),
    promptMetadata,
  )
}

function applyOutlineTurnExternalMetadata(
  entry: SessionHistoryTranscriptEntry,
  metadata: OutlineTurnExternalMetadata | null,
): SessionHistoryTranscriptEntry {
  return applyExternalProviderObservedTurnMetadata({ ...entry }, metadata)
}

function transcriptEntryExternalMetadata(
  entry: SessionHistoryTranscriptEntry,
): OutlineTurnExternalMetadata | null {
  return transcriptExternalProviderObservedTurnMetadata(entry)
}

function applyOutlineTurnLifecycleMetadata(
  entry: SessionHistoryTranscriptEntry,
  completedAtMs: number | null | undefined,
): SessionHistoryTranscriptEntry {
  if (completedAtMs === undefined) {
    return entry
  }
  const lifecycle = transcriptHistoryTurnLifecycleFromCompletedAtMs(completedAtMs)
  return {
    ...entry,
    historyTurnCompletedAtMs: completedAtMs,
    ...(lifecycle !== undefined ? { historyTurnLifecycle: lifecycle } : {}),
  }
}

function transcriptHistoryTurnIsOpen(entry: SessionHistoryTranscriptEntry): boolean {
  return entry.historyTurnLifecycle === "open"
}

function hydrateSessionHistoryPageEntriesForTurn(
  pageEntries: SessionHistoryPageEntry[],
  turnId?: number,
  promptId?: string | null,
): SessionHistoryTranscriptEntry[] {
  const hydrateOptions = promptId === undefined ? {} : { promptId }
  return hydrateSessionHistoryTranscriptEntries(pageEntries, hydrateOptions).map((entry) => ({
    ...entry,
    ...(turnId !== undefined ? { turnId } : {}),
  }))
}

function outlineBlobTranscriptEntry(
  blob: SessionHistoryOutlineBlob,
  agentId: string,
  turnId: number,
  promptId: string | null,
  id: number,
): SessionHistoryTranscriptEntry {
  return {
    id,
    role: sessionHistoryOutlineBlobTranscriptRole(blob.kind),
    text: "",
    sourceText: "",
    turnId,
    promptId,
    blobCollapsible: true,
    blobCollapsed: true,
    blobTitle: blob.title,
    blobSummary: blob.summary,
    historyBlobId: blob.blob_id,
    historyBlobAgentId: agentId,
    historyBlobLoaded: false,
    historyEntryIndex: blob.sequence_start,
    historyFragmentStart: 0,
    historyFragmentEnd: blob.total_chars,
    historyTotalChars: blob.total_chars,
  }
}

function sessionHistoryOutlineBlobTranscriptRole(
  kind: SessionHistoryOutlineBlob["kind"],
): SessionHistoryTranscriptEntry["role"] {
  if (kind === "notice") {
    return "status"
  }
  const role = providerTranscriptRoleForKind(kind)
  return role === "reasoning" || role === "error" || role === "status"
    ? role
    : "tool"
}
