import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
} from "./transcript-display.js"

export type AgentPaneTranscriptInteractionControllerDeps = {
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  expandedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  setExpandedTurnState: (
    agentId: string | null | undefined,
    turnId: number | null | undefined,
    expanded: boolean,
  ) => void
  commitAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  reconcileMountedAuxiliaryTranscript: (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
  ) => void
  retainPromptFocus: () => void
}

export function createAgentPaneTranscriptInteractionController(
  deps: AgentPaneTranscriptInteractionControllerDeps,
) {
  const toggleTurn = (
    agentId: string,
    turnId: number | null | undefined,
    toggleEntryId?: number,
  ) => {
    if (!turnId) {
      return
    }
    const currentEntries = deps.currentAgentPaneEntries(agentId)
    const toggleEntry = resolveVisibleTurnToggle(currentEntries, turnId, toggleEntryId)
    if (!toggleEntry) {
      return
    }

    const expanding = toggleEntry.toggleMode === "expand"
    deps.setExpandedTurnState(agentId, turnId, expanding)
    const nextEntries = applyTranscriptDisplayState(
      currentEntries,
      expanding
        ? deps.expandedTurnIdsForAgent(agentId).filter((value) => value !== turnId)
        : [...deps.expandedTurnIdsForAgent(agentId), turnId],
    )
    commitAndRefocus(deps, agentId, currentEntries, nextEntries)
  }

  const toggleBlob = (agentId: string, entryId: number, collapsed: boolean) => {
    const currentEntries = deps.currentAgentPaneEntries(agentId)
    const nextEntries = setTranscriptBlobCollapsed(
      currentEntries,
      entryId,
      deps.expandedTurnIdsForAgent(agentId),
      collapsed,
    )
    commitAndRefocus(deps, agentId, currentEntries, nextEntries)
  }

  return {
    toggleBlob,
    toggleTurn,
  }
}

function commitAndRefocus(
  deps: Pick<
    AgentPaneTranscriptInteractionControllerDeps,
    "commitAgentPaneEntries" | "reconcileMountedAuxiliaryTranscript" | "retainPromptFocus"
  >,
  agentId: string,
  currentEntries: TranscriptEntry[],
  nextEntries: TranscriptEntry[],
) {
  deps.commitAgentPaneEntries(agentId, nextEntries)
  deps.reconcileMountedAuxiliaryTranscript(agentId, currentEntries, nextEntries)
  deps.retainPromptFocus()
}
