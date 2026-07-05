import type { TranscriptEntry } from "./cli-types.js"
import type { SessionHistoryBlobContent } from "./cli-types.js"
import {
  markSessionHistoryBlobLoading,
  replaceSessionHistoryBlobPlaceholder,
} from "@arroba/kernel-client/session-history-transcript"
import {
  applyTranscriptDisplayState,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
} from "@arroba/kernel-client/transcript-display-state"
import { shouldSkipConsecutiveTranscriptEntry } from "./transcript.js"

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
    const preparedEntries = applyTranscriptDisplayState(nextEntries, turnIds)
    deps.setEntries(preparedEntries)
    deps.setEntryCounter(maxTranscriptEntryId(preparedEntries))
    return preparedEntries
  }

  const toggleTurn = (turnId: number | null | undefined, toggleEntryId?: number) => {
    if (!turnId) {
      return
    }
    const currentEntries = deps.entries().filter(Boolean)
    const toggleEntry = resolveVisibleTurnToggle(currentEntries, turnId, toggleEntryId)
    if (!toggleEntry) {
      return
    }
    const agentId = deps.visibleTranscriptAgentId()
    const expanding = toggleEntry.toggleMode === "expand"
    deps.setExpandedTurnState(agentId, turnId, expanding)
    const nextEntries = applyTranscriptDisplayState(currentEntries, expanding
      ? deps.expandedTurnIdsForAgent(agentId).filter((value) => value !== turnId)
      : [...deps.expandedTurnIdsForAgent(agentId), turnId])
    deps.setEntries(nextEntries)
    deps.setEntryCounter(maxTranscriptEntryId(nextEntries))
    deps.persistVisibleTranscriptEntries(nextEntries)
    deps.reconcileMountedTranscript(currentEntries, nextEntries)
    deps.retainPromptFocus()
  }

  const toggleBlob = (entryId: number, collapsed: boolean) => {
    const currentEntries = deps.entries().filter(Boolean)
    const agentId = deps.visibleTranscriptAgentId()
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
      deps.setEntries(loadingEntries)
      deps.persistVisibleTranscriptEntries(loadingEntries)
      deps.reconcileMountedTranscript(currentEntries, loadingEntries)
      deps.retainPromptFocus()
      void deps.loadHistoryBlobContent(target.historyBlobAgentId, target.historyBlobId)
        .then((content) => {
          const latestEntries = deps.entries().filter(Boolean)
          const nextEntries = replaceSessionHistoryBlobPlaceholder(
            latestEntries,
            entryId,
            content,
            deps.expandedTurnIdsForAgent(agentId),
          ) as TranscriptEntry[]
          deps.setEntries(nextEntries)
          deps.setEntryCounter(maxTranscriptEntryId(nextEntries))
          deps.persistVisibleTranscriptEntries(nextEntries)
          deps.reconcileMountedTranscript(latestEntries, nextEntries)
          deps.retainPromptFocus()
        })
        .catch((error) => {
          const latestEntries = deps.entries().filter(Boolean)
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
    const nextEntries = setTranscriptBlobCollapsed(
      currentEntries,
      entryId,
      deps.expandedTurnIdsForAgent(agentId),
      collapsed,
    )
    deps.setEntries(nextEntries)
    deps.setEntryCounter(maxTranscriptEntryId(nextEntries))
    deps.persistVisibleTranscriptEntries(nextEntries)
    deps.reconcileMountedTranscript(currentEntries, nextEntries)
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
    const currentEntries = deps.entries().filter(Boolean)
    const nextId = deps.entryCounter() + 1
    const nextEntry: TranscriptEntry = { id: nextId, ...entry }
    const currentTurnId = deps.currentTurnId()
    if (nextEntry.turnId === undefined && currentTurnId !== null) {
      nextEntry.turnId = currentTurnId
    }
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

function maxTranscriptEntryId(entries: readonly TranscriptEntry[]) {
  return entries.reduce((max, entry) => Math.max(max, entry.id), 0)
}
