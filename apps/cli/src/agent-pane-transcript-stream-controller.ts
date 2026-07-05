import type { TranscriptEntry } from "./cli-types.js"
import {
  type ToolTranscriptUpdate,
} from "./transcript.js"
import {
  applyTranscriptProviderChunk,
  applyTranscriptToolUpdate,
  type TranscriptStreamMetadata,
} from "@arroba/kernel-client/transcript-stream-state"

export type AgentPaneTranscriptStreamControllerDeps = {
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  trimLiveAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => TranscriptEntry[]
  setAgentTranscriptEntries: (agentId: string, entries: TranscriptEntry[]) => void
  commitStreamingAgentPaneEntry: (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
    updatedEntryId: number,
  ) => void
  toolStateForAgent: (agentId: string) => Map<string, ToolTranscriptUpdate>
}

export function createAgentPaneTranscriptStreamController(
  deps: AgentPaneTranscriptStreamControllerDeps,
) {
  const appendProviderChunk = (
    agentId: string,
    role: TranscriptEntry["role"],
    chunk: string,
    mergeKey?: string,
    sourceText?: string,
    metadata: TranscriptStreamMetadata = {},
  ) => {
    const currentEntries = deps.currentAgentPaneEntries(agentId).map((entry) => ({ ...entry }))
    const result = applyTranscriptProviderChunk(currentEntries, {
      role,
      chunk,
      mergeKey,
      sourceText,
      metadata,
    })
    if (result.kind === "noop") {
      return
    }

    const nextEntries = result.entries as TranscriptEntry[]
    if (result.kind === "merged" && result.updatedEntryId !== undefined) {
      deps.commitStreamingAgentPaneEntry(agentId, currentEntries, nextEntries, result.updatedEntryId)
      return
    }

    deps.setAgentTranscriptEntries(
      agentId,
      deps.trimLiveAgentPaneEntries(agentId, nextEntries),
    )
  }

  const appendToolUpdate = (agentId: string, chunk: string, metadata: TranscriptStreamMetadata = {}) => {
    const currentEntries = deps.currentAgentPaneEntries(agentId).map((entry) => ({ ...entry }))
    const result = applyTranscriptToolUpdate(currentEntries, chunk, deps.toolStateForAgent(agentId), metadata)
    if (result.kind === "noop") {
      return
    }

    const nextEntries = result.entries as TranscriptEntry[]
    if (result.kind === "merged" && result.updatedEntryId !== undefined) {
      deps.commitStreamingAgentPaneEntry(agentId, currentEntries, nextEntries, result.updatedEntryId)
      return
    }

    deps.setAgentTranscriptEntries(
      agentId,
      deps.trimLiveAgentPaneEntries(agentId, nextEntries),
    )
  }

  return {
    appendProviderChunk,
    appendToolUpdate,
  }
}
