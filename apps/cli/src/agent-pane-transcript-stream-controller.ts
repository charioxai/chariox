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

export function createAgentPaneTranscriptStreamController(
  deps: AgentPaneTranscriptStreamControllerDeps,
) {
  const appendProviderChunk = (
    agentId: string,
    role: TranscriptEntry["role"],
    chunk: string,
    mergeKey?: string,
    sourceText?: string,
  ) => {
    const normalized = normalizeProviderChunk(chunk)
    const normalizedSource = sourceText === undefined ? undefined : normalizeProviderChunk(sourceText)
    if (!normalized) {
      return
    }

    const currentEntries = deps.currentAgentPaneEntries(agentId).map((entry) => ({ ...entry }))
    const nextEntries = currentEntries.map((entry) => ({ ...entry }))
    const mergedEntry = mergeProviderChunk(nextEntries, {
      role,
      normalized,
      normalizedSource,
      mergeKey,
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
    }))
    deps.setAgentTranscriptEntries(
      agentId,
      deps.trimLiveAgentPaneEntries(agentId, nextEntries),
    )
  }

  const appendToolUpdate = (agentId: string, chunk: string) => {
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
      )
      return
    }

    appendProviderChunk(agentId, "tool", normalized, undefined, normalized)
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
  },
) {
  const { role, normalized, normalizedSource, mergeKey } = options

  if (mergeKey) {
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const candidate = entries[index]
      if (candidate?.role !== role || candidate.mergeKey !== mergeKey) {
        continue
      }
      applyMergedChunk(candidate, role, normalized, normalizedSource)
      return candidate
    }
  }

  const last = [...entries].reverse().find((entry) => entry.role !== "turn_toggle")
  if (!mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
    last.text += normalized
    return last
  }

  return null
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
  return nextEntry
}
