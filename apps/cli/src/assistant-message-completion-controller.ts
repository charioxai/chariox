import type { TranscriptEntry } from "./cli-types.js"
import {
  cloneCompactTranscriptDisplayEntries,
  projectSettledTranscriptTurnDisplayState,
} from "@arroba/kernel-client/transcript-display-state"

export type AssistantMessageCompletionControllerDeps = {
  entries: () => TranscriptEntry[]
  visibleTranscriptAgentId: () => string | null | undefined
  splitAgentResponseMode: () => boolean
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  collapsedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  setCollapsedTurnIdsForAgent: (agentId: string, turnIds: number[]) => void
  setEntries: (entries: TranscriptEntry[]) => void
  setEntryCounter: (value: number) => void
  persistVisibleTranscriptEntries: (entries: TranscriptEntry[]) => void
  reconcileMountedTranscript: (currentEntries: TranscriptEntry[], nextEntries: TranscriptEntry[]) => void
  setAgentTranscriptEntries: (
    agentId: string,
    entries: TranscriptEntry[],
    turnIds?: readonly number[],
  ) => void
  clearAgentBusy: (agentId: string | null | undefined) => void
  confirmTurnCompletion: () => void
  maybeScheduleConfirmedTurnCompletion: () => void
}

export function createAssistantMessageCompletionController(
  deps: AssistantMessageCompletionControllerDeps,
) {
  const markCompleted = (agentId: string | null | undefined) => {
    const completionAgentId = agentId ?? deps.visibleTranscriptAgentId()
    const currentEntries = completionAgentId
      && deps.splitAgentResponseMode()
      && completionAgentId !== deps.visibleTranscriptAgentId()
      ? cloneCompactTranscriptDisplayEntries(deps.currentAgentPaneEntries(completionAgentId))
      : cloneCompactTranscriptDisplayEntries(deps.entries())

    if (completionAgentId) {
      const projection = projectSettledTranscriptTurnDisplayState(
        currentEntries,
        deps.collapsedTurnIdsForAgent(completionAgentId),
      )

      if (projection.settledTurnId !== null) {
        deps.setCollapsedTurnIdsForAgent(completionAgentId, projection.collapsedTurnIds)
      }

      if (projection.settledTurnId !== null && completionAgentId === deps.visibleTranscriptAgentId()) {
        deps.setEntries(projection.entries)
        deps.setEntryCounter(projection.entryCounter)
        deps.persistVisibleTranscriptEntries(projection.entries)
        deps.reconcileMountedTranscript(currentEntries, projection.entries)
      } else if (projection.settledTurnId !== null) {
        deps.setAgentTranscriptEntries(
          completionAgentId,
          projection.entries,
          projection.collapsedTurnIds,
        )
      }
    }

    deps.clearAgentBusy(completionAgentId)
    deps.confirmTurnCompletion()
    deps.maybeScheduleConfirmedTurnCompletion()
  }

  return {
    markCompleted,
  }
}
