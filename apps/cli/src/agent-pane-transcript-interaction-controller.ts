import type {
  SessionHistoryBlobContent,
  TranscriptEntry,
} from "./cli-types.js"
import {
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
} from "@arroba/kernel-client/session-history-transcript"
import {
  applyTranscriptDisplayState,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
} from "@arroba/kernel-client/transcript-display-state"

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
  loadHistoryBlobContent?: (agentId: string, blobId: string) => Promise<SessionHistoryBlobContent>
  formatError?: (error: unknown) => string
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
    const target = currentEntries.find((entry) => entry.id === entryId)
    if (
      collapsed === false
      && target?.historyBlobId
      && target.historyBlobLoaded !== true
      && target.historyBlobLoading !== true
      && target.historyBlobAgentId
      && deps.loadHistoryBlobContent
    ) {
      const loadingEntries = markSessionHistoryBlobLoading(currentEntries, entryId, true) as TranscriptEntry[]
      commitAndRefocus(deps, agentId, currentEntries, loadingEntries)
      void deps.loadHistoryBlobContent(target.historyBlobAgentId, target.historyBlobId)
        .then((content) => {
          const latestEntries = deps.currentAgentPaneEntries(agentId)
          const nextEntries = replaceSessionHistoryBlobPlaceholder(
            latestEntries,
            entryId,
            content,
            deps.expandedTurnIdsForAgent(agentId),
          ) as TranscriptEntry[]
          commitAndRefocus(deps, agentId, latestEntries, nextEntries)
        })
        .catch((error) => {
          const latestEntries = deps.currentAgentPaneEntries(agentId)
          const nextEntries = markSessionHistoryBlobLoading(
            latestEntries,
            entryId,
            false,
            deps.formatError?.(error) ?? String(error),
          ) as TranscriptEntry[]
          commitAndRefocus(deps, agentId, latestEntries, nextEntries)
        })
      return
    }
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
