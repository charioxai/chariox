import {
  describeCollapsedTranscriptBlob,
  type CollapsedTranscriptBlobEntry,
  type CollapsedTranscriptBlobDescription,
} from "./transcript-collapsed-blob.js"
import {
  stripTranscriptDisplayOnlyEntries,
  transcriptEntryIsBlobCollapsible,
  transcriptTurnFinalAssistantEntry,
  transcriptTurnHasCollapsibleBody,
  transcriptTurnIsCollapsible,
  type TranscriptRoleEntry,
  type TranscriptTurnDisplayEntry,
} from "./transcript-entry-lineage.js"

export type TranscriptDisplayEntry = TranscriptTurnDisplayEntry & TranscriptRoleEntry & {
  readonly text: string
  readonly sourceText?: string | null
  readonly historyTurnCompletedAtMs?: number | null
  readonly turnTracking?: "none" | string | null
  readonly hidden?: boolean
  readonly blobCollapsible?: boolean
  readonly blobCollapsed?: boolean
  readonly blobTitle?: string
  readonly blobSummary?: string
  readonly toggleMode?: "expand" | "collapse"
}

export type TranscriptBlobDescription = CollapsedTranscriptBlobDescription

export type TranscriptDisplayStateOptions = {
  readonly describeCollapsedBlob?: (entry: TranscriptDisplayEntry) => TranscriptBlobDescription
}

type MutableTranscriptDisplayEntry =
  Omit<TranscriptDisplayEntry, "turnId" | "hidden" | "blobCollapsible" | "blobCollapsed" | "blobTitle" | "blobSummary" | "toggleMode"> & {
  turnId?: number | null | undefined
  hidden?: boolean
  blobCollapsible?: boolean
  blobCollapsed?: boolean
  blobTitle?: string
  blobSummary?: string
  toggleMode?: "expand" | "collapse"
}

export function normalizeTranscriptTurnIds<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
): TEntry[] {
  let activeTurnId: number | undefined
  let nextTurnId = 1

  return entries.map((entry) => {
    const next: MutableTranscriptDisplayEntry = { ...entry }
    if (entry.turnTracking === "none") {
      delete next.turnId
      return next as TEntry
    }
    if (entry.role === "user") {
      activeTurnId = entry.turnId ?? nextTurnId
      next.turnId = activeTurnId
      nextTurnId = Math.max(nextTurnId, activeTurnId + 1)
      return next as TEntry
    }
    if (activeTurnId !== undefined) {
      next.turnId = activeTurnId
    }
    return next as TEntry
  })
}

export function stripTranscriptDisplayEntries<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
): TEntry[] {
  return stripTranscriptDisplayOnlyEntries(entries)
}

export function collapseLatestTranscriptTurn<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  collapsedTurnIds: readonly number[] = [],
): number[] {
  const nextCollapsedTurnIds = new Set(collapsedTurnIds)
  const normalized = normalizeTranscriptTurnIds(stripTranscriptDisplayEntries(entries))
  const turnIds = [...new Set(normalized.map((entry) => entry.turnId).filter((turnId): turnId is number => typeof turnId === "number"))]
  const latestTurnId = turnIds.at(-1)
  if (latestTurnId === undefined) {
    return sortedTurnIds(nextCollapsedTurnIds)
  }

  const turnEntries = normalized.filter((entry) => entry.turnId === latestTurnId)
  if (!transcriptTurnIsCollapsible(turnEntries, inferredActiveHistoryTurnId(normalized))) {
    return sortedTurnIds(nextCollapsedTurnIds)
  }

  nextCollapsedTurnIds.add(latestTurnId)
  return sortedTurnIds(nextCollapsedTurnIds)
}

export function applyTranscriptDisplayState<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  collapsedTurnIds: readonly number[] = [],
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TEntry[] {
  const normalized = normalizeTranscriptTurnIds(stripTranscriptDisplayEntries(entries)).map((entry) => ({
    ...entry,
    hidden: false,
  })) as MutableTranscriptDisplayEntry[]
  const collapsedTurnIdSet = new Set(collapsedTurnIds)
  let nextId = normalized.reduce((max, entry) => Math.max(max, entry.id), 0)
  const turnIds = [...new Set(normalized.map((entry) => entry.turnId).filter((turnId): turnId is number => typeof turnId === "number"))]
  const effectiveActiveTurnId = activeTurnId ?? inferredActiveHistoryTurnId(normalized)

  for (const turnId of turnIds) {
    const turnEntries = normalized.filter((entry) => entry.turnId === turnId)
    const finalSummary = transcriptTurnFinalAssistantEntry(turnEntries)
    const collapsibleTurn = transcriptTurnIsCollapsible(turnEntries, effectiveActiveTurnId)
    const expanded = collapsibleTurn ? !collapsedTurnIdSet.has(turnId) : false

    for (const entry of turnEntries) {
      const blobCollapsible = transcriptEntryIsBlobCollapsible(entry)
      if (blobCollapsible) {
        entry.blobCollapsible = true
        entry.blobCollapsed = entry.blobCollapsed ?? true
        if (!entry.historyBlobId) {
          const preview = options.describeCollapsedBlob
            ? options.describeCollapsedBlob(entry)
            : describeCollapsedTranscriptBlob(entry as CollapsedTranscriptBlobEntry)
          if (preview) {
            entry.blobTitle = preview.title
            entry.blobSummary = preview.summary
          }
        }
      } else {
        entry.blobCollapsible = false
        delete entry.blobCollapsed
        delete entry.blobTitle
        delete entry.blobSummary
      }
      if (!collapsibleTurn || expanded) {
        entry.hidden = false
        continue
      }
      entry.hidden = !(entry.role === "user" || entry.id === finalSummary!.id)
    }

    if (!collapsibleTurn) {
      continue
    }

    const promptIndex = normalized.findIndex((entry) => entry.turnId === turnId && entry.role === "user")
    const anchorIndex = promptIndex >= 0
      ? promptIndex
      : normalized.findIndex((entry) => entry.turnId === turnId)
    if (anchorIndex === -1) {
      continue
    }

    normalized.splice(anchorIndex + 1, 0, {
      id: ++nextId,
      role: "turn_toggle",
      text: expanded ? "click to collapse" : "click to expand",
      turnId,
      hidden: false,
      toggleMode: expanded ? "collapse" : "expand",
      blobCollapsible: false,
    } as MutableTranscriptDisplayEntry)
  }

  return normalized as TEntry[]
}

function inferredActiveHistoryTurnId(
  entries: readonly TranscriptDisplayEntry[],
): number | null {
  const activeTurnIds = entries
    .filter((entry) => entry.historyTurnCompletedAtMs === null)
    .map((entry) => entry.turnId)
    .filter((turnId): turnId is number => typeof turnId === "number")
  return activeTurnIds.at(-1) ?? null
}

export function setTranscriptTurnExpanded<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  turnId: number,
  collapsedTurnIds: readonly number[],
  expanded: boolean,
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TranscriptDisplayEntry[] {
  const nextCollapsedTurnIds = new Set(collapsedTurnIds)
  if (expanded) {
    nextCollapsedTurnIds.delete(turnId)
  } else {
    nextCollapsedTurnIds.add(turnId)
  }
  return applyTranscriptDisplayState(entries, sortedTurnIds(nextCollapsedTurnIds), activeTurnId, options)
}

export function setTranscriptBlobCollapsed<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  entryId: number,
  collapsedTurnIds: readonly number[] = [],
  collapsed: boolean,
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TranscriptDisplayEntry[] {
  const updated = stripTranscriptDisplayEntries(entries).map((entry) => {
    if (entry.id !== entryId) {
      return { ...entry }
    }
    return {
      ...entry,
      blobCollapsed: collapsed,
    }
  }) as TranscriptDisplayEntry[]
  return applyTranscriptDisplayState(updated, collapsedTurnIds, activeTurnId, options)
}

export function findVisibleTurnToggle<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  turnId: number | null | undefined,
  toggleEntryId?: number,
): TEntry | undefined {
  if (!turnId) {
    return undefined
  }
  return entries.find((entry) => {
    if (!entry || entry.turnId !== turnId || entry.role !== "turn_toggle" || entry.hidden) {
      return false
    }
    return toggleEntryId === undefined || entry.id === toggleEntryId
  })
}

export function resolveVisibleTurnToggle<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  turnId: number | null | undefined,
  preferredToggleEntryId?: number,
): TEntry | undefined {
  return findVisibleTurnToggle(entries, turnId, preferredToggleEntryId)
    ?? findVisibleTurnToggle(entries, turnId)
}

function sortedTurnIds(turnIds: Iterable<number>) {
  return [...turnIds].sort((left, right) => left - right)
}
