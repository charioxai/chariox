import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
  replaceCollapsedTranscriptTurnIds,
  updateCollapsedTranscriptTurnState,
  type CollapsedTranscriptTurnIdsByAgent,
} from "@arroba/kernel-client/transcript-display-state"

export type CollapsedTurnIdsByAgent = CollapsedTranscriptTurnIdsByAgent
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
      updateCollapsedTranscriptTurnState(current, agentId, turnId, expanded))
  }

  const replaceExpandedTurnsForAgent = (
    agentId: string | null | undefined,
    turnIds: readonly number[],
  ) => {
    if (!agentId) {
      return
    }
    deps.updateExpandedTurnIdsByAgent((current) =>
      replaceCollapsedTranscriptTurnIds(current, agentId, turnIds))
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

export const applyExpandedTurns = applyCollapsedTurns
export const updateExpandedTurnState = updateCollapsedTranscriptTurnState
export const replaceExpandedTurnIds = replaceCollapsedTranscriptTurnIds
