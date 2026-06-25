import type { TranscriptEntry } from "./cli-types.js"
import { computeCurrentTurnId } from "./transcript-preview.js"
import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "./transcript.js"

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

type TranscriptStreamMetadata = {
  promptId?: string | null
  sourceAttachmentId?: string | null
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
    const normalized = normalizeProviderChunk(chunk)
    const normalizedSource = sourceText === undefined ? undefined : normalizeProviderChunk(sourceText)
    if (!normalized) {
      return
    }

    const currentEntries = deps.currentAgentPaneEntries(agentId).map((entry) => ({ ...entry }))
    const nextEntries = currentEntries.map((entry) => ({ ...entry }))
    const currentTurnId = computeCurrentTurnId(nextEntries)
    const mergedEntry = mergeProviderChunk(nextEntries, {
      role,
      normalized,
      normalizedSource,
      mergeKey,
      metadata,
      currentTurnId,
    })

    if (mergedEntry) {
      deps.commitStreamingAgentPaneEntry(agentId, currentEntries, nextEntries, mergedEntry.id)
      return
    }

    nextEntries.push(createTranscriptEntry(nextEntries, {
      role,
      normalized,
      normalizedSource,
      mergeKey,
      metadata,
    }))
    deps.setAgentTranscriptEntries(
      agentId,
      deps.trimLiveAgentPaneEntries(agentId, nextEntries),
    )
  }

  const appendToolUpdate = (agentId: string, chunk: string, metadata: TranscriptStreamMetadata = {}) => {
    const normalized = normalizeProviderChunk(chunk)
    if (!normalized) {
      return
    }

    const parsed = parseToolTranscriptUpdate(normalized)
    if (parsed) {
      const toolState = deps.toolStateForAgent(agentId)
      const merged = mergeToolTranscriptUpdate(toolState.get(parsed.id) ?? null, parsed)
      toolState.set(parsed.id, merged)
      appendProviderChunk(
        agentId,
        "tool",
        formatToolTranscriptUpdate(merged),
        parsed.id,
        JSON.stringify(merged),
        metadata,
      )
      return
    }

    appendProviderChunk(agentId, "tool", normalized, undefined, normalized, metadata)
  }

  return {
    appendProviderChunk,
    appendToolUpdate,
  }
}

function normalizeProviderChunk(chunk: string) {
  return chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
}

function mergeProviderChunk(
  entries: TranscriptEntry[],
  options: {
    role: TranscriptEntry["role"]
    normalized: string
    normalizedSource: string | undefined
    mergeKey: string | undefined
    metadata: TranscriptStreamMetadata
    currentTurnId: number | null
  },
) {
  const { role, normalized, normalizedSource, mergeKey, metadata, currentTurnId } = options

  if (mergeKey) {
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const candidate = entries[index]
      if (
        candidate?.role !== role
        || candidate.mergeKey !== mergeKey
        || !sameStreamingTurn(candidate, currentTurnId)
      ) {
        continue
      }
      applyMergedChunk(candidate, role, normalized, normalizedSource)
      applyStreamMetadata(candidate, metadata)
      return candidate
    }
  }

  const last = [...entries].reverse().find((entry) => entry.role !== "turn_toggle")
  if (
    !mergeKey
    && last?.role === role
    && sameStreamingTurn(last, currentTurnId)
    && (role === "assistant" || role === "reasoning")
  ) {
    last.text += normalized
    applyStreamMetadata(last, metadata)
    return last
  }

  return null
}

function sameStreamingTurn(entry: TranscriptEntry, currentTurnId: number | null) {
  return currentTurnId === null || entry.turnId === currentTurnId
}

function applyMergedChunk(
  candidate: TranscriptEntry,
  role: TranscriptEntry["role"],
  normalized: string,
  normalizedSource: string | undefined,
) {
  if (role === "assistant" || role === "reasoning") {
    candidate.text += normalized
    if (normalizedSource !== undefined) {
      candidate.sourceText = `${candidate.sourceText ?? ""}${normalizedSource}`
    }
    return
  }

  candidate.text = normalized
  if (normalizedSource !== undefined) {
    candidate.sourceText = normalizedSource
  }
}

function createTranscriptEntry(
  entries: TranscriptEntry[],
  options: {
    role: TranscriptEntry["role"]
    normalized: string
    normalizedSource: string | undefined
    mergeKey: string | undefined
    metadata: TranscriptStreamMetadata
  },
) {
  const nextEntry: TranscriptEntry = {
    id: entries.reduce((max, entry) => Math.max(max, entry.id), 0) + 1,
    role: options.role,
    text: options.normalized,
  }
  const currentTurnId = computeCurrentTurnId(entries)
  if (currentTurnId !== null) {
    nextEntry.turnId = currentTurnId
  }
  if (options.mergeKey) {
    nextEntry.mergeKey = options.mergeKey
  }
  if (options.normalizedSource !== undefined) {
    nextEntry.sourceText = options.normalizedSource
  }
  applyStreamMetadata(nextEntry, options.metadata)
  return nextEntry
}

function applyStreamMetadata(entry: TranscriptEntry, metadata: TranscriptStreamMetadata) {
  if (metadata.promptId !== undefined) {
    entry.promptId = metadata.promptId
  }
  if (metadata.sourceAttachmentId !== undefined) {
    entry.sourceAttachmentId = metadata.sourceAttachmentId
  }
}
