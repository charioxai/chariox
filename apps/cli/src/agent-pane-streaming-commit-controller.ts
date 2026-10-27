import type { TranscriptEntry } from "./cli-types.js"
import { projectCompactTranscriptDisplayState } from "@chariox/kernel-client/transcript-display-state"

export type AgentPaneStreamingCommitControllerDeps = {
  trimLiveAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => TranscriptEntry[]
  collapsedTurnIdsForAgent: (agentId: string) => readonly number[]
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
    const retainedEntries = deps.trimLiveAgentPaneEntries(agentId, nextEntries)
    const sanitizedEntries = projectCompactTranscriptDisplayState(
      retainedEntries,
      deps.collapsedTurnIdsForAgent(agentId),
    ).entries
    deps.commitAgentPaneEntries(agentId, sanitizedEntries)
    if (deps.splitAgentResponseMode() && agentId === deps.getResponsePrimaryAgentId()) {
      deps.replaceTranscriptEntries(sanitizedEntries.map((entry) => ({ ...entry })), agentId)
      return
    }
    if (!deps.splitAgentResponseMode() || !deps.visibleAuxiliaryAgentIds().includes(agentId)) {
      return
    }
    const updatedEntry = sanitizedEntries.find((entry) => entry.id === updatedEntryId)
    // Retention can evict siblings while keeping the streamed entry. Reconcile
    // those removals before taking the single-entry render update shortcut.
    if (updatedEntry && retainedEntries.length === nextEntries.length) {
      deps.updateAuxiliaryTranscriptEntry(agentId, updatedEntry)
      return
    }
    deps.reconcileMountedAuxiliaryTranscript(agentId, currentEntries, sanitizedEntries)
  }

  return {
    commitStreamingEntry,
  }
}
