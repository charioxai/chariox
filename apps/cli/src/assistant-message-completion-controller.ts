import type { TranscriptEntry } from "./cli-types.js"
import { projectTranscriptDisplayState } from "@arroba/kernel-client/transcript-display-state"
import { computeCurrentTranscriptTurnId as computeCurrentTurnId } from "@arroba/kernel-client/transcript-entry-state"

export type AssistantMessageCompletionControllerDeps = {
  entries: () => TranscriptEntry[]
  visibleTranscriptAgentId: () => string | null | undefined
  splitAgentResponseMode: () => boolean
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  expandedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  setExpandedTurnIdsForAgent: (agentId: string, turnIds: number[]) => void
  setEntries: (entries: TranscriptEntry[]) => void
  setEntryCounter: (value: number) => void
  persistVisibleTranscriptEntries: (entries: TranscriptEntry[]) => void
  reconcileMountedTranscript: (currentEntries: TranscriptEntry[], nextEntries: TranscriptEntry[]) => void
  setAgentTranscriptEntries: (agentId: string, entries: TranscriptEntry[]) => void
  clearAgentBusy: (agentId: string | null | undefined) => void
  confirmTurnCompletion: () => void
  maybeScheduleConfirmedTurnCompletion: () => void
}

export function createAssistantMessageCompletionController(
  deps: AssistantMessageCompletionControllerDeps,
) {
  const markCompleted = (agentId: string | null | undefined) => {
    const completionAgentId = agentId ?? deps.visibleTranscriptAgentId()
    const turnId = completionAgentId && deps.splitAgentResponseMode() && completionAgentId !== deps.visibleTranscriptAgentId()
      ? computeCurrentTurnId(deps.currentAgentPaneEntries(completionAgentId))
      : computeCurrentTurnId(deps.entries().filter(Boolean))

    if (completionAgentId && turnId !== null) {
      const nextExpandedTurnIds = [...new Set([...deps.expandedTurnIdsForAgent(completionAgentId), turnId])]
        .filter((value) => value !== turnId)
        .sort((left, right) => left - right)
      deps.setExpandedTurnIdsForAgent(completionAgentId, nextExpandedTurnIds)

      if (completionAgentId === deps.visibleTranscriptAgentId()) {
        const currentEntries = deps.entries().filter(Boolean)
        const projection = projectTranscriptDisplayState(currentEntries, nextExpandedTurnIds)
        deps.setEntries(projection.entries)
        deps.setEntryCounter(projection.entryCounter)
        deps.persistVisibleTranscriptEntries(projection.entries)
        deps.reconcileMountedTranscript(currentEntries, projection.entries)
      } else {
        deps.setAgentTranscriptEntries(completionAgentId, deps.currentAgentPaneEntries(completionAgentId))
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
