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
import {
  computeCurrentTranscriptTurnId,
  computeMaxTranscriptEntryId,
  computeNextTranscriptTurnId,
} from "./transcript-entry-state.js"
import type { TranscriptEntry as KernelTranscriptEntry } from "./kernel-types.js"

type TranscriptDisplayKernelFields = Pick<
  KernelTranscriptEntry,
  | "text"
  | "hidden"
  | "blobCollapsible"
  | "blobCollapsed"
  | "blobTitle"
  | "blobSummary"
  | "toggleMode"
>

export type TranscriptDisplayEntry = TranscriptTurnDisplayEntry & TranscriptRoleEntry & TranscriptDisplayKernelFields & {
  readonly sourceText?: KernelTranscriptEntry["sourceText"] | null
  readonly historyTurnCompletedAtMs?: number | null
  readonly historyTurnLifecycle?: TranscriptHistoryTurnLifecycle
  readonly turnTracking?: "none" | string | null
}

export type TranscriptHistoryTurnLifecycle = "open" | "completed"

export type TranscriptBlobDescription = CollapsedTranscriptBlobDescription

export type TranscriptDisplayStateOptions = {
  readonly describeCollapsedBlob?: (entry: TranscriptDisplayEntry) => TranscriptBlobDescription
}

export type TranscriptDisplayProjection<TEntry extends TranscriptDisplayEntry> = {
  readonly entries: TEntry[]
  readonly currentTurnId: number | null
  readonly nextTurnId: number
  readonly entryCounter: number
}

export type TranscriptDisplayEntryInput<TEntry extends TranscriptDisplayEntry> =
  | TEntry
  | null
  | undefined
  | false

export type TranscriptTurnSettlementProjection<TEntry extends TranscriptDisplayEntry> =
  TranscriptDisplayProjection<TEntry> & {
    readonly settledTurnId: number | null
    readonly collapsedTurnIds: number[]
  }

export type TranscriptTurnToggleProjection<TEntry extends TranscriptDisplayEntry> =
  TranscriptDisplayProjection<TEntry> & {
    readonly turnId: number
    readonly expanded: boolean
    readonly collapsedTurnIds: number[]
  }

export type TranscriptBlobToggleProjection<TEntry extends TranscriptDisplayEntry> =
  TranscriptDisplayProjection<TEntry> & {
    readonly entryId: number
    readonly collapsed: boolean
  }

export type CollapsedTranscriptTurnIdsByAgent = Record<string, number[]>

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

export function compactTranscriptDisplayEntries<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TranscriptDisplayEntryInput<TEntry>[],
): TEntry[] {
  return entries.filter((entry): entry is TEntry => Boolean(entry))
}

export function cloneCompactTranscriptDisplayEntries<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TranscriptDisplayEntryInput<TEntry>[],
): TEntry[] {
  return compactTranscriptDisplayEntries(entries).map((entry) => ({ ...entry }))
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
  if (
    openHistoryTurnIds(normalized).has(latestTurnId)
    || !transcriptTurnIsCollapsible(turnEntries)
  ) {
    return sortedTurnIds(nextCollapsedTurnIds)
  }

  nextCollapsedTurnIds.add(latestTurnId)
  return sortedTurnIds(nextCollapsedTurnIds)
}

export function updateCollapsedTranscriptTurnState(
  current: CollapsedTranscriptTurnIdsByAgent,
  agentId: string,
  turnId: number,
  expanded: boolean,
) {
  const previous = new Set(current[agentId] ?? [])
  if (expanded) {
    previous.delete(turnId)
  } else {
    previous.add(turnId)
  }
  return replaceCollapsedTranscriptTurnIds(current, agentId, previous)
}

export function replaceCollapsedTranscriptTurnIds(
  current: CollapsedTranscriptTurnIdsByAgent,
  agentId: string,
  turnIds: Iterable<number>,
) {
  const nextTurnIds = sortedTurnIds(new Set(turnIds))
  if (nextTurnIds.length === 0) {
    if (!(agentId in current)) {
      return current
    }
    const next = { ...current }
    delete next[agentId]
    return next
  }

  const currentTurnIds = current[agentId] ?? []
  if (
    currentTurnIds.length === nextTurnIds.length
    && currentTurnIds.every((value, index) => value === nextTurnIds[index])
  ) {
    return current
  }
  return {
    ...current,
    [agentId]: nextTurnIds,
  }
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
  const activeHistoryTurnIds = openHistoryTurnIds(normalized)

  for (const turnId of turnIds) {
    const turnEntries = normalized.filter((entry) => entry.turnId === turnId)
    const finalSummary = transcriptTurnFinalAssistantEntry(turnEntries)
    const collapsibleTurn = !activeHistoryTurnIds.has(turnId)
      && transcriptTurnIsCollapsible(turnEntries, activeTurnId)
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

export function projectTranscriptDisplayState<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  collapsedTurnIds: readonly number[] = [],
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TranscriptDisplayProjection<TEntry> {
  const projectedEntries = applyTranscriptDisplayState(entries, collapsedTurnIds, activeTurnId, options)
  return {
    entries: projectedEntries,
    currentTurnId: computeCurrentTranscriptTurnId(projectedEntries),
    nextTurnId: computeNextTranscriptTurnId(projectedEntries),
    entryCounter: computeMaxTranscriptEntryId(projectedEntries),
  }
}

export function projectCompactTranscriptDisplayState<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TranscriptDisplayEntryInput<TEntry>[],
  collapsedTurnIds: readonly number[] = [],
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TranscriptDisplayProjection<TEntry> {
  return projectTranscriptDisplayState(
    compactTranscriptDisplayEntries(entries),
    collapsedTurnIds,
    activeTurnId,
    options,
  )
}

export function projectSettledTranscriptTurnDisplayState<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  collapsedTurnIds: readonly number[] = [],
  options: TranscriptDisplayStateOptions = {},
): TranscriptTurnSettlementProjection<TEntry> {
  const normalized = normalizeTranscriptTurnIds(stripTranscriptDisplayEntries(entries))
  const settledTurnId = computeCurrentTranscriptTurnId(normalized)
  const nextCollapsedTurnIds = new Set(collapsedTurnIds)
  if (settledTurnId !== null) {
    const activeHistoryTurnIds = openHistoryTurnIds(normalized)
    const turnEntries = normalized.filter((entry) => entry.turnId === settledTurnId)
    if (
      !activeHistoryTurnIds.has(settledTurnId)
      && transcriptTurnIsCollapsible(turnEntries)
    ) {
      nextCollapsedTurnIds.add(settledTurnId)
    } else {
      nextCollapsedTurnIds.delete(settledTurnId)
    }
  }
  const projection = projectTranscriptDisplayState(
    normalized,
    sortedTurnIds(nextCollapsedTurnIds),
    null,
    options,
  )
  return {
    ...projection,
    settledTurnId,
    collapsedTurnIds: sortedTurnIds(nextCollapsedTurnIds),
  }
}

export function projectTranscriptTurnToggleDisplayState<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  turnId: number | null | undefined,
  collapsedTurnIds: readonly number[] = [],
  preferredToggleEntryId?: number,
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TranscriptTurnToggleProjection<TEntry> | null {
  if (turnId === null || turnId === undefined) {
    return null
  }
  const toggleEntry = resolveVisibleTurnToggle(entries, turnId, preferredToggleEntryId)
  if (!toggleEntry) {
    return null
  }
  const expanded = toggleEntry.toggleMode === "expand"
  const nextCollapsedTurnIds = new Set(collapsedTurnIds)
  if (expanded) {
    nextCollapsedTurnIds.delete(turnId)
  } else {
    nextCollapsedTurnIds.add(turnId)
  }
  const projection = projectTranscriptDisplayState(
    entries,
    sortedTurnIds(nextCollapsedTurnIds),
    activeTurnId,
    options,
  )
  return {
    ...projection,
    turnId,
    expanded,
    collapsedTurnIds: sortedTurnIds(nextCollapsedTurnIds),
  }
}

export function projectTranscriptBlobToggleDisplayState<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  entryId: number,
  collapsedTurnIds: readonly number[] = [],
  collapsed: boolean,
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TranscriptBlobToggleProjection<TEntry> | null {
  if (!entries.some((entry) => entry.id === entryId)) {
    return null
  }
  const projection = projectTranscriptDisplayState(
    stripTranscriptDisplayEntries(entries).map((entry) => {
      if (entry.id !== entryId) {
        return { ...entry }
      }
      return {
        ...entry,
        blobCollapsed: collapsed,
      }
    }) as TEntry[],
    collapsedTurnIds,
    activeTurnId,
    options,
  )
  return {
    ...projection,
    entryId,
    collapsed,
  }
}

function openHistoryTurnIds(
  entries: readonly TranscriptDisplayEntry[],
): ReadonlySet<number> {
  const lifecycleByTurnId = new Map<number, { completed: boolean; open: boolean }>()
  for (const entry of entries) {
    if (typeof entry.turnId !== "number") {
      continue
    }
    const lifecycleName = entry.historyTurnLifecycle
    if (lifecycleName === undefined) {
      continue
    }
    const lifecycle = lifecycleByTurnId.get(entry.turnId) ?? { completed: false, open: false }
    if (lifecycleName === "completed") {
      lifecycle.completed = true
    } else {
      lifecycle.open = true
    }
    lifecycleByTurnId.set(entry.turnId, lifecycle)
  }
  return new Set([...lifecycleByTurnId.entries()]
    .filter(([, lifecycle]) => lifecycle.open && !lifecycle.completed)
    .map(([turnId]) => turnId))
}

export function setTranscriptTurnExpanded<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  turnId: number,
  collapsedTurnIds: readonly number[],
  expanded: boolean,
  activeTurnId: number | null = null,
  options: TranscriptDisplayStateOptions = {},
): TEntry[] {
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
): TEntry[] {
  return projectTranscriptBlobToggleDisplayState(
    entries,
    entryId,
    collapsedTurnIds,
    collapsed,
    activeTurnId,
    options,
  )?.entries ?? applyTranscriptDisplayState(entries, collapsedTurnIds, activeTurnId, options)
}

export function findVisibleTurnToggle<TEntry extends TranscriptDisplayEntry>(
  entries: readonly TEntry[],
  turnId: number | null | undefined,
  toggleEntryId?: number,
): TEntry | undefined {
  if (turnId === null || turnId === undefined) {
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
