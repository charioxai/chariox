import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
  replaceCollapsedTranscriptTurnIds,
  updateCollapsedTranscriptTurnState,
  type CollapsedTranscriptTurnIdsByAgent,
} from "@arroba/kernel-client/transcript-display-state"

export type CollapsedTurnIdsByAgent = CollapsedTranscriptTurnIdsByAgent

export type TranscriptTurnCollapseControllerDeps = {
  collapsedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  updateCollapsedTurnIdsByAgent: (
    updater: (current: CollapsedTurnIdsByAgent) => CollapsedTurnIdsByAgent,
  ) => void
}

export function createTranscriptTurnCollapseController(deps: TranscriptTurnCollapseControllerDeps) {
  const setExpandedTurnState = (
    agentId: string | null | undefined,
    turnId: number | null | undefined,
    expanded: boolean,
  ) => {
    if (!agentId || turnId === null || turnId === undefined) {
      return
    }
    deps.updateCollapsedTurnIdsByAgent((current) =>
      updateCollapsedTranscriptTurnState(current, agentId, turnId, expanded))
  }

  const replaceCollapsedTurnsForAgent = (
    agentId: string | null | undefined,
    turnIds: readonly number[],
  ) => {
    if (!agentId) {
      return
    }
    deps.updateCollapsedTurnIdsByAgent((current) =>
      replaceCollapsedTranscriptTurnIds(current, agentId, turnIds))
  }

  const collapseLatestTurnForAgent = (
    agentId: string | null | undefined,
    paneEntries: TranscriptEntry[],
  ) => {
    const nextTurnIds = collapseLatestTranscriptTurn(
      paneEntries,
      deps.collapsedTurnIdsForAgent(agentId),
    )
    replaceCollapsedTurnsForAgent(agentId, nextTurnIds)
    return nextTurnIds
  }

  return {
    applyCollapsedTurns: applyCollapsedTurns,
    collapseLatestTurnForAgent,
    setExpandedTurnState,
  }
}

export function applyCollapsedTurns(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[],
) {
  return applyTranscriptDisplayState(entries, collapsedTurnIds)
}
