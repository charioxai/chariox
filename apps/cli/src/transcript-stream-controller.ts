import type { TranscriptEntry } from "./cli-types.js"
import { getToolActivityLabel } from "./runtime.js"
import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "./transcript.js"

export type TranscriptStreamControllerDeps = {
  entries: () => TranscriptEntry[]
  setEntries: (entries: TranscriptEntry[]) => void
  entryCounter: () => number
  currentTurnId: () => number | null
  tools: Map<string, ToolTranscriptUpdate>
  activeToolLabels: Map<string, string>
  cancelPendingTurnCompletion: () => void
  setWorking: (value: boolean) => void
  setSubmitting: (value: boolean) => void
  updateSessionChrome: () => void
  syncVisibleActivityLabel: () => void
  applyVisibleTranscriptState: (entries: TranscriptEntry[]) => TranscriptEntry[]
  persistVisibleTranscriptEntries: (entries: TranscriptEntry[]) => void
  reconcileMountedTranscript: (currentEntries: TranscriptEntry[], nextEntries: TranscriptEntry[]) => void
  updateTranscriptEntry: (entryId: number, text: string, sourceText?: string) => void
  logVisibleTranscriptOutput: (
    role: TranscriptEntry["role"],
    text: string,
    merged: boolean,
    mergeKey?: string,
  ) => void
  enforceTranscriptRetention: () => void
  maybeScheduleConfirmedTurnCompletion: () => void
}

export function createTranscriptStreamController(deps: TranscriptStreamControllerDeps) {
  const syncActiveToolLabel = (update: ToolTranscriptUpdate) => {
    const label = getToolActivityLabel(update.tool)
    const terminal = update.status === "completed" || update.status === "error" || update.status === "cancelled"

    deps.activeToolLabels.delete(update.id)
    if (label && !terminal) {
      deps.activeToolLabels.set(update.id, label)
    }

    deps.syncVisibleActivityLabel()
  }

  const appendProviderChunk = (
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

    deps.cancelPendingTurnCompletion()
    deps.setWorking(true)
    deps.setSubmitting(false)

    const currentEntries = deps.entries().filter(Boolean).map((entry) => ({ ...entry }))
    const nextEntries = currentEntries.map((entry) => ({ ...entry }))
    const mergedEntry = mergeProviderChunk(nextEntries, {
      role,
      normalized,
      normalizedSource,
      mergeKey,
    })

    if (mergedEntry) {
      deps.setEntries(nextEntries)
      deps.persistVisibleTranscriptEntries(nextEntries)
      deps.updateTranscriptEntry(mergedEntry.id, mergedEntry.text, mergedEntry.sourceText)
      deps.logVisibleTranscriptOutput(role, mergedEntry.text, true, mergeKey)
      deps.enforceTranscriptRetention()
      deps.maybeScheduleConfirmedTurnCompletion()
      return
    }

    const nextEntry: TranscriptEntry = {
      id: deps.entryCounter() + 1,
      role,
      text: normalized,
    }
    const currentTurnId = deps.currentTurnId()
    if (currentTurnId !== null) {
      nextEntry.turnId = currentTurnId
    }
    if (mergeKey) {
      nextEntry.mergeKey = mergeKey
    }
    if (normalizedSource !== undefined) {
      nextEntry.sourceText = normalizedSource
    }
    nextEntries.push(nextEntry)

    const preparedEntries = deps.applyVisibleTranscriptState(nextEntries)
    deps.persistVisibleTranscriptEntries(preparedEntries)
    deps.reconcileMountedTranscript(currentEntries, preparedEntries)
    const loggedEntry = [...preparedEntries].reverse().find((entry) => entry.role === role && (mergeKey ? entry.mergeKey === mergeKey : true))
    deps.logVisibleTranscriptOutput(role, loggedEntry?.text ?? normalized, false, mergeKey)
    deps.enforceTranscriptRetention()
    deps.maybeScheduleConfirmedTurnCompletion()
  }

  const appendToolUpdate = (chunk: string) => {
    const normalized = normalizeProviderChunk(chunk)
    if (!normalized) {
      return
    }

    deps.cancelPendingTurnCompletion()
    deps.setWorking(true)
    deps.updateSessionChrome()

    const parsed = parseToolTranscriptUpdate(normalized)
    if (parsed) {
      const merged = mergeToolTranscriptUpdate(deps.tools.get(parsed.id) ?? null, parsed)
      deps.tools.set(parsed.id, merged)
      syncActiveToolLabel(merged)
      appendProviderChunk("tool", formatToolTranscriptUpdate(merged), parsed.id, JSON.stringify(merged))
      return
    }

    appendProviderChunk("tool", normalized, undefined, normalized)
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
      if (role === "assistant" || role === "reasoning") {
        candidate.text += normalized
        if (normalizedSource !== undefined) {
          candidate.sourceText = `${candidate.sourceText ?? ""}${normalizedSource}`
        }
      } else {
        candidate.text = normalized
        if (normalizedSource !== undefined) {
          candidate.sourceText = normalizedSource
        }
      }
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
