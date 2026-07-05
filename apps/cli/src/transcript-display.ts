import {
  applyTranscriptDisplayState as sharedApplyTranscriptDisplayState,
  collapseLatestTranscriptTurn as sharedCollapseLatestTranscriptTurn,
  findVisibleTurnToggle as sharedFindVisibleTurnToggle,
  normalizeTranscriptTurnIds as sharedNormalizeTranscriptTurnIds,
  resolveVisibleTurnToggle as sharedResolveVisibleTurnToggle,
  setTranscriptBlobCollapsed as sharedSetTranscriptBlobCollapsed,
  setTranscriptTurnExpanded as sharedSetTranscriptTurnExpanded,
  stripTranscriptDisplayEntries as sharedStripTranscriptDisplayEntries,
} from "@arroba/kernel-client/transcript-display-state"
import { describeCollapsedTranscriptBlob } from "@arroba/kernel-client/transcript-collapsed-blob"
import type { TranscriptEntry } from "./cli-types.js"

export function normalizeTranscriptTurnIds(entries: TranscriptEntry[]) {
  return sharedNormalizeTranscriptTurnIds(entries) as TranscriptEntry[]
}

export function stripTranscriptDisplayEntries(entries: TranscriptEntry[]) {
  return sharedStripTranscriptDisplayEntries(entries)
}

export function collapseLatestTranscriptTurn(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[] = [],
) {
  return sharedCollapseLatestTranscriptTurn(entries, collapsedTurnIds)
}

export function applyTranscriptDisplayState(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[] = [],
  activeTurnId: number | null = null,
) {
  return sharedApplyTranscriptDisplayState(entries, collapsedTurnIds, activeTurnId, {
    describeCollapsedBlob: describeCollapsedTranscriptBlob,
  }) as TranscriptEntry[]
}

export function setTranscriptTurnExpanded(
  entries: TranscriptEntry[],
  turnId: number,
  collapsedTurnIds: readonly number[],
  expanded: boolean,
  activeTurnId: number | null = null,
) {
  return sharedSetTranscriptTurnExpanded(entries, turnId, collapsedTurnIds, expanded, activeTurnId, {
    describeCollapsedBlob: describeCollapsedTranscriptBlob,
  }) as TranscriptEntry[]
}

export function setTranscriptBlobCollapsed(
  entries: TranscriptEntry[],
  entryId: number,
  collapsedTurnIds: readonly number[] = [],
  collapsed: boolean,
  activeTurnId: number | null = null,
) {
  return sharedSetTranscriptBlobCollapsed(entries, entryId, collapsedTurnIds, collapsed, activeTurnId, {
    describeCollapsedBlob: describeCollapsedTranscriptBlob,
  }) as TranscriptEntry[]
}

export function findVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  toggleEntryId?: number,
) {
  return sharedFindVisibleTurnToggle(entries, turnId, toggleEntryId)
}

export function resolveVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  preferredToggleEntryId?: number,
) {
  return sharedResolveVisibleTurnToggle(entries, turnId, preferredToggleEntryId)
}
