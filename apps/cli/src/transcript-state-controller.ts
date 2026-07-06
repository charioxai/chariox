import type { TranscriptEntry } from "./cli-types.js"
import type { SessionHistoryBlobContent } from "./cli-types.js"
import {
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
  resolveSessionHistoryBlobLoadTarget,
} from "@arroba/kernel-client/session-history-transcript"
import {
  compactTranscriptDisplayEntries,
  projectCompactTranscriptDisplayState,
  projectTranscriptBlobToggleDisplayState,
  projectTranscriptTurnToggleDisplayState,
} from "@arroba/kernel-client/transcript-display-state"
import {
  computeMaxTranscriptEntryId,
  createNextTranscriptEntry,
  shouldSkipConsecutiveTranscriptEntry,
  transcriptEntryRuntimeOptions,
} from "@arroba/kernel-client/transcript-entry-state"

export type TranscriptStateControllerDeps = {
  entries: () => TranscriptEntry[]
  setEntries: (entries: TranscriptEntry[]) => void
  entryCounter: () => number
  setEntryCounter: (value: number) => void
  currentTurnId: () => number | null
  visibleTranscriptAgentId: () => string | null | undefined
  expandedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  setExpandedTurnState: (agentId: string | null | undefined, turnId: number, expanded: boolean) => void
  persistVisibleTranscriptEntries: (entries: TranscriptEntry[]) => void
  reconcileMountedTranscript: (currentEntries: TranscriptEntry[], nextEntries: TranscriptEntry[]) => void
  retainPromptFocus: () => void
  enforceTranscriptRetention: () => void
  loadHistoryBlobContent?: (agentId: string, blobId: string) => Promise<SessionHistoryBlobContent>
  formatError?: (error: unknown) => string
}

export function createTranscriptStateController(deps: TranscriptStateControllerDeps) {
  const applyVisibleState = (
    nextEntries: TranscriptEntry[],
    agentId: string | null | undefined = deps.visibleTranscriptAgentId(),
    turnIds = deps.expandedTurnIdsForAgent(agentId),
  ) => {
    const projection = projectCompactTranscriptDisplayState(nextEntries, turnIds)
    deps.setEntries(projection.entries)
    deps.setEntryCounter(projection.entryCounter)
    return projection.entries
  }

  const toggleTurn = (turnId: number | null | undefined, toggleEntryId?: number) => {
    if (!turnId) {
      return
    }
    const currentEntries = compactTranscriptDisplayEntries(deps.entries())
    const agentId = deps.visibleTranscriptAgentId()
    const projection = projectTranscriptTurnToggleDisplayState(
      currentEntries,
      turnId,
      deps.expandedTurnIdsForAgent(agentId),
      toggleEntryId,
    )
    if (!projection) {
      return
    }
    deps.setExpandedTurnState(agentId, turnId, projection.expanded)
    deps.setEntries(projection.entries)
    deps.setEntryCounter(projection.entryCounter)
    deps.persistVisibleTranscriptEntries(projection.entries)
    deps.reconcileMountedTranscript(currentEntries, projection.entries)
    deps.retainPromptFocus()
  }

  const toggleBlob = (entryId: number, collapsed: boolean) => {
    const currentEntries = compactTranscriptDisplayEntries(deps.entries())
    const agentId = deps.visibleTranscriptAgentId()
    const target = currentEntries.find((entry) => entry.id === entryId)
    const loadTarget = resolveSessionHistoryBlobLoadTarget(target, collapsed)
    if (loadTarget && deps.loadHistoryBlobContent) {
      const loadingEntries = markSessionHistoryBlobLoading(currentEntries, entryId, true) as TranscriptEntry[]
      deps.setEntries(loadingEntries)
      deps.persistVisibleTranscriptEntries(loadingEntries)
      deps.reconcileMountedTranscript(currentEntries, loadingEntries)
      deps.retainPromptFocus()
      void deps.loadHistoryBlobContent(loadTarget.agentId, loadTarget.blobId)
        .then((content) => {
          const latestEntries = compactTranscriptDisplayEntries(deps.entries())
          const nextEntries = replaceSessionHistoryBlobPlaceholder(
            latestEntries,
            entryId,
            content,
            deps.expandedTurnIdsForAgent(agentId),
          ) as TranscriptEntry[]
          deps.setEntries(nextEntries)
          deps.setEntryCounter(computeMaxTranscriptEntryId(nextEntries))
          deps.persistVisibleTranscriptEntries(nextEntries)
          deps.reconcileMountedTranscript(latestEntries, nextEntries)
          deps.retainPromptFocus()
        })
        .catch((error) => {
          const latestEntries = compactTranscriptDisplayEntries(deps.entries())
          const nextEntries = markSessionHistoryBlobLoading(
            latestEntries,
            entryId,
            false,
            deps.formatError?.(error) ?? String(error),
          ) as TranscriptEntry[]
          deps.setEntries(nextEntries)
          deps.persistVisibleTranscriptEntries(nextEntries)
          deps.reconcileMountedTranscript(latestEntries, nextEntries)
          deps.retainPromptFocus()
        })
      return
    }
    const projection = projectTranscriptBlobToggleDisplayState(
      currentEntries,
      entryId,
      deps.expandedTurnIdsForAgent(agentId),
      collapsed,
    )
    if (!projection) {
      return
    }
    deps.setEntries(projection.entries)
    deps.setEntryCounter(projection.entryCounter)
    deps.persistVisibleTranscriptEntries(projection.entries)
    deps.reconcileMountedTranscript(currentEntries, projection.entries)
    deps.retainPromptFocus()
  }

  const appendEntry = (
    entry: Omit<TranscriptEntry, "id">,
    turnIds = deps.expandedTurnIdsForAgent(deps.visibleTranscriptAgentId()),
  ) => {
    const previousEntry = deps.entries().at(-1)
    if (shouldSkipConsecutiveTranscriptEntry(previousEntry, entry)) {
      return null
    }
    const currentEntries = compactTranscriptDisplayEntries(deps.entries())
    const runtimeOptions = transcriptEntryRuntimeOptions({
      entryCounter: deps.entryCounter(),
      currentTurnId: deps.currentTurnId(),
    })
    const nextEntry = createNextTranscriptEntry<TranscriptEntry, Omit<TranscriptEntry, "id">>(
      currentEntries,
      entry,
      runtimeOptions,
    )
    const nextEntries = applyVisibleState(
      [...currentEntries, nextEntry],
      deps.visibleTranscriptAgentId(),
      turnIds,
    )
    deps.persistVisibleTranscriptEntries(nextEntries)
    deps.reconcileMountedTranscript(currentEntries, nextEntries)
    deps.enforceTranscriptRetention()
    return nextEntry
  }

  return {
    appendEntry,
    applyVisibleState,
    toggleBlob,
    toggleTurn,
  }
}
