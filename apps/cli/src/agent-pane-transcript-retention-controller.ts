import type { TranscriptEntry } from "./cli-types.js"
import { trimAgentPaneEntries } from "./agent-pane-state.js"

export type AgentPaneTranscriptRetentionControllerDeps = {
  maxEntries: number
  maxChars: number
  deleteToolForMergeKey: (agentId: string, mergeKey: string) => void
}

export function createAgentPaneTranscriptRetentionController(
  deps: AgentPaneTranscriptRetentionControllerDeps,
) {
  const trimLiveEntries = (agentId: string, entries: TranscriptEntry[]) => trimAgentPaneEntries({
    entries,
    maxEntries: deps.maxEntries,
    maxChars: deps.maxChars,
    onTrimmedMergeKey: (mergeKey) => {
      deps.deleteToolForMergeKey(agentId, mergeKey)
    },
  })

  return {
    trimLiveEntries,
  }
}
