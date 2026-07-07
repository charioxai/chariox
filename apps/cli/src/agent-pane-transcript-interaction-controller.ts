import type {
  SessionHistoryBlobContent,
  TranscriptEntry,
} from "./cli-types.js"
import {
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
  resolveSessionHistoryBlobLoadTarget,
} from "@arroba/kernel-client/session-history-transcript"
import {
  projectTranscriptBlobToggleDisplayState,
  projectTranscriptTurnToggleDisplayState,
} from "@arroba/kernel-client/transcript-display-state"

export type AgentPaneTranscriptInteractionControllerDeps = {
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  collapsedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
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
    if (turnId === null || turnId === undefined) {
      return
    }
    const currentEntries = deps.currentAgentPaneEntries(agentId)
    const projection = projectTranscriptTurnToggleDisplayState(
      currentEntries,
      turnId,
      deps.collapsedTurnIdsForAgent(agentId),
      toggleEntryId,
    )
    if (!projection) {
      return
    }

    deps.setExpandedTurnState(agentId, turnId, projection.expanded)
    commitAndRefocus(deps, agentId, currentEntries, projection.entries)
  }

  const toggleBlob = (agentId: string, entryId: number, collapsed: boolean) => {
    const currentEntries = deps.currentAgentPaneEntries(agentId)
    const target = currentEntries.find((entry) => entry.id === entryId)
    const loadTarget = resolveSessionHistoryBlobLoadTarget(target, collapsed)
    if (loadTarget && deps.loadHistoryBlobContent) {
      const loadingEntries = markSessionHistoryBlobLoading(currentEntries, entryId, true) as TranscriptEntry[]
      commitAndRefocus(deps, agentId, currentEntries, loadingEntries)
      void deps.loadHistoryBlobContent(loadTarget.agentId, loadTarget.blobId)
        .then((content) => {
          const latestEntries = deps.currentAgentPaneEntries(agentId)
          const nextEntries = replaceSessionHistoryBlobPlaceholder(
            latestEntries,
            entryId,
            content,
            deps.collapsedTurnIdsForAgent(agentId),
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
    const projection = projectTranscriptBlobToggleDisplayState(
      currentEntries,
      entryId,
      deps.collapsedTurnIdsForAgent(agentId),
      collapsed,
    )
    if (!projection) {
      return
    }
    commitAndRefocus(deps, agentId, currentEntries, projection.entries)
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
