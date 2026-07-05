import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
} from "@arroba/kernel-client/transcript-display-state"

export type CollapsedTurnIdsByAgent = Record<string, number[]>
export type ExpandedTurnIdsByAgent = CollapsedTurnIdsByAgent

export type TranscriptTurnExpansionControllerDeps = {
  expandedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  updateExpandedTurnIdsByAgent: (
    updater: (current: ExpandedTurnIdsByAgent) => ExpandedTurnIdsByAgent,
  ) => void
}

export function createTranscriptTurnExpansionController(deps: TranscriptTurnExpansionControllerDeps) {
  const setExpandedTurnState = (
    agentId: string | null | undefined,
    turnId: number | null | undefined,
    expanded: boolean,
  ) => {
    if (!agentId || !turnId) {
      return
    }
    deps.updateExpandedTurnIdsByAgent((current) =>
      updateCollapsedTurnState(current, agentId, turnId, expanded))
  }

  const replaceExpandedTurnsForAgent = (
    agentId: string | null | undefined,
    turnIds: readonly number[],
  ) => {
    if (!agentId) {
      return
    }
    deps.updateExpandedTurnIdsByAgent((current) =>
      replaceCollapsedTurnIds(current, agentId, turnIds))
  }

  const collapseLatestTurnForAgent = (
    agentId: string | null | undefined,
    paneEntries: TranscriptEntry[],
  ) => {
    const nextTurnIds = collapseLatestTranscriptTurn(
      paneEntries,
      deps.expandedTurnIdsForAgent(agentId),
    )
    replaceExpandedTurnsForAgent(agentId, nextTurnIds)
    return nextTurnIds
  }

  return {
    applyExpandedTurns: applyCollapsedTurns,
    collapseLatestTurnForAgent,
    replaceExpandedTurnsForAgent,
    setExpandedTurnState,
  }
}

export function applyCollapsedTurns(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[],
) {
  return applyTranscriptDisplayState(entries, collapsedTurnIds)
}

export function updateCollapsedTurnState(
  current: CollapsedTurnIdsByAgent,
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
  return replaceCollapsedTurnIds(current, agentId, previous)
}

export function replaceCollapsedTurnIds(
  current: CollapsedTurnIdsByAgent,
  agentId: string,
  turnIds: Iterable<number>,
) {
  const nextTurnIds = [...new Set(turnIds)].sort((left, right) => left - right)
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

export const applyExpandedTurns = applyCollapsedTurns
export const updateExpandedTurnState = updateCollapsedTurnState
export const replaceExpandedTurnIds = replaceCollapsedTurnIds
