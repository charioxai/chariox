import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
} from "./transcript-display.js"

export type ExpandedTurnIdsByAgent = Record<string, number[]>

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
      updateExpandedTurnState(current, agentId, turnId, expanded))
  }

  const replaceExpandedTurnsForAgent = (
    agentId: string | null | undefined,
    turnIds: readonly number[],
  ) => {
    if (!agentId) {
      return
    }
    deps.updateExpandedTurnIdsByAgent((current) =>
      replaceExpandedTurnIds(current, agentId, turnIds))
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
    applyExpandedTurns,
    collapseLatestTurnForAgent,
    replaceExpandedTurnsForAgent,
    setExpandedTurnState,
  }
}

export function applyExpandedTurns(
  entries: TranscriptEntry[],
  expandedTurnIds: readonly number[],
) {
  return applyTranscriptDisplayState(entries, expandedTurnIds)
}

export function updateExpandedTurnState(
  current: ExpandedTurnIdsByAgent,
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
  return replaceExpandedTurnIds(current, agentId, previous)
}

export function replaceExpandedTurnIds(
  current: ExpandedTurnIdsByAgent,
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
