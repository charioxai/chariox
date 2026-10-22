import { toggleTranscriptDetails } from "./transcript-details-controller.js"
import type {
  SessionHistoryBlobContent,
  TranscriptEntry,
} from "./cli-types.js"
import {
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
  resolveSessionHistoryBlobLoadTarget,
} from "@chariox/kernel-client/session-history-transcript"
import {
  cloneCompactTranscriptDisplayEntries,
  projectTranscriptBlobToggleDisplayState,
  projectTranscriptTurnToggleDisplayState,
} from "@chariox/kernel-client/transcript-display-state"

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
  loadHistoryBlobContents?: (agentId: string, blobIds: readonly string[]) => Promise<SessionHistoryBlobContent[]>
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
    const currentEntries = cloneCompactTranscriptDisplayEntries(deps.currentAgentPaneEntries(agentId))
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
    const currentEntries = cloneCompactTranscriptDisplayEntries(deps.currentAgentPaneEntries(agentId))
    const target = currentEntries.find((entry) => entry.id === entryId)
    const loadTarget = resolveSessionHistoryBlobLoadTarget(target, collapsed)
    if (loadTarget && deps.loadHistoryBlobContent) {
      const loadingEntries = markSessionHistoryBlobLoading(currentEntries, loadTarget.blobId, true) as TranscriptEntry[]
      commitAndRefocus(deps, agentId, currentEntries, loadingEntries)
      void deps.loadHistoryBlobContent(loadTarget.agentId, loadTarget.blobId)
        .then((content) => {
          const latestEntries = cloneCompactTranscriptDisplayEntries(deps.currentAgentPaneEntries(agentId))
          const nextEntries = replaceSessionHistoryBlobPlaceholder(
            latestEntries,
            loadTarget.blobId,
            content,
            deps.collapsedTurnIdsForAgent(agentId),
          ) as TranscriptEntry[]
          commitAndRefocus(deps, agentId, latestEntries, nextEntries)
        })
        .catch((error) => {
          const latestEntries = cloneCompactTranscriptDisplayEntries(deps.currentAgentPaneEntries(agentId))
          const nextEntries = markSessionHistoryBlobLoading(
            latestEntries,
            loadTarget.blobId,
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

  const toggleAllBlobs = (agentId: string) => toggleTranscriptDetails({
    entries: () => deps.currentAgentPaneEntries(agentId),
    collapsedTurns: () => deps.collapsedTurnIdsForAgent(agentId),
    load: deps.loadHistoryBlobContents,
    formatError: deps.formatError,
    commit: (current, next) => commitAndRefocus(deps, agentId, current, next),
  })
  return {
    toggleAllBlobs,
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
