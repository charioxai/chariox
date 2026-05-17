import type { TranscriptEntry } from "./cli-types.js"
import { applyTranscriptDisplayState } from "./transcript-display.js"

export type AgentPaneStreamingCommitControllerDeps = {
  trimLiveAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => TranscriptEntry[]
  expandedTurnIdsForAgent: (agentId: string) => readonly number[]
  commitAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => void
  splitAgentResponseMode: () => boolean
  getResponsePrimaryAgentId: () => string | null | undefined
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string) => void
  visibleAuxiliaryAgentIds: () => readonly string[]
  updateAuxiliaryTranscriptEntry: (agentId: string, entry: TranscriptEntry) => void
  reconcileMountedAuxiliaryTranscript: (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
  ) => void
}

export function createAgentPaneStreamingCommitController(
  deps: AgentPaneStreamingCommitControllerDeps,
) {
  const commitStreamingEntry = (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
    updatedEntryId: number,
  ) => {
    const sanitizedEntries = applyTranscriptDisplayState(
      deps.trimLiveAgentPaneEntries(agentId, nextEntries).filter(Boolean),
      deps.expandedTurnIdsForAgent(agentId),
    )
    deps.commitAgentPaneEntries(agentId, sanitizedEntries)
    if (deps.splitAgentResponseMode() && agentId === deps.getResponsePrimaryAgentId()) {
      deps.replaceTranscriptEntries(sanitizedEntries.map((entry) => ({ ...entry })), agentId)
      return
    }
    if (!deps.splitAgentResponseMode() || !deps.visibleAuxiliaryAgentIds().includes(agentId)) {
      return
    }
    const updatedEntry = sanitizedEntries.find((entry) => entry.id === updatedEntryId)
    if (updatedEntry) {
      deps.updateAuxiliaryTranscriptEntry(agentId, updatedEntry)
      return
    }
    deps.reconcileMountedAuxiliaryTranscript(agentId, currentEntries, sanitizedEntries)
  }

  return {
    commitStreamingEntry,
  }
}
