import type { TranscriptEntry } from "./cli-types.js"
import { cloneCompactTranscriptDisplayEntries } from "@arroba/kernel-client/transcript-display-state"
import { getToolActivityLabel } from "@arroba/kernel-client/provider-status"
import {
  applyTranscriptProviderChunk,
  applyTranscriptToolUpdate,
  transcriptStreamRuntimeOptions,
  transcriptStreamRuntimeTransition,
  type TranscriptStreamApplyResult,
  type TranscriptStreamMetadata,
} from "@arroba/kernel-client/transcript-stream-state"
import type { ToolTranscriptUpdate } from "./transcript.js"

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
    metadata: TranscriptStreamMetadata = {},
  ) => {
    const currentEntries = cloneCompactTranscriptDisplayEntries(deps.entries())
    const runtimeOptions = transcriptStreamRuntimeOptions({
      entryCounter: deps.entryCounter(),
      currentTurnId: deps.currentTurnId(),
    })
    const result = applyTranscriptProviderChunk(currentEntries, {
      role,
      chunk,
      mergeKey,
      sourceText,
      metadata,
      ...runtimeOptions,
    })
    if (result.kind === "noop") {
      return
    }

    const transition = applyStreamRuntimeActivity(result)

    commitStreamResult(role, currentEntries, result, mergeKey)
    deps.enforceTranscriptRetention()
    if (transition.shouldScheduleConfirmedTurnCompletion) {
      deps.maybeScheduleConfirmedTurnCompletion()
    }
  }

  const appendToolUpdate = (chunk: string, metadata: TranscriptStreamMetadata = {}) => {
    const currentEntries = cloneCompactTranscriptDisplayEntries(deps.entries())
    const runtimeOptions = transcriptStreamRuntimeOptions({
      entryCounter: deps.entryCounter(),
      currentTurnId: deps.currentTurnId(),
    })
    const result = applyTranscriptToolUpdate(
      currentEntries,
      chunk,
      deps.tools,
      metadata,
      runtimeOptions,
    )
    if (result.kind === "noop") {
      return
    }

    const transition = applyStreamRuntimeActivity(result)
    deps.updateSessionChrome()

    if (result.mergedUpdate) {
      syncActiveToolLabel(result.mergedUpdate)
    }

    const updatedEntry = findUpdatedEntry(result.entries as TranscriptEntry[], result.updatedEntryId)
    commitStreamResult("tool", currentEntries, result, updatedEntry?.mergeKey ?? undefined)
    deps.enforceTranscriptRetention()
    if (transition.shouldScheduleConfirmedTurnCompletion) {
      deps.maybeScheduleConfirmedTurnCompletion()
    }
  }

  const applyStreamRuntimeActivity = (result: TranscriptStreamApplyResult<TranscriptEntry>) => {
    const transition = transcriptStreamRuntimeTransition(result)
    if (transition.shouldCancelPendingTurnCompletion) {
      deps.cancelPendingTurnCompletion()
    }
    if (transition.working !== null) {
      deps.setWorking(transition.working)
    }
    if (transition.submitting !== null) {
      deps.setSubmitting(transition.submitting)
    }
    return transition
  }

  const commitStreamResult = (
    role: TranscriptEntry["role"],
    currentEntries: TranscriptEntry[],
    result: TranscriptStreamApplyResult<TranscriptEntry>,
    mergeKey: string | undefined,
  ): void => {
    const nextEntries = result.entries as TranscriptEntry[]
    const updatedEntry = findUpdatedEntry(nextEntries, result.updatedEntryId)

    if (result.kind === "merged" && updatedEntry) {
      deps.setEntries(nextEntries)
      deps.persistVisibleTranscriptEntries(nextEntries)
      deps.updateTranscriptEntry(updatedEntry.id, updatedEntry.text, updatedEntry.sourceText)
      deps.logVisibleTranscriptOutput(role, updatedEntry.text, true, mergeKey)
      return
    }

    const preparedEntries = deps.applyVisibleTranscriptState(nextEntries)
    deps.persistVisibleTranscriptEntries(preparedEntries)
    deps.reconcileMountedTranscript(currentEntries, preparedEntries)
    const loggedEntry = findUpdatedEntry(preparedEntries, result.updatedEntryId)
      ?? [...preparedEntries].reverse().find((entry) => entry.role === role && (mergeKey ? entry.mergeKey === mergeKey : true))
    deps.logVisibleTranscriptOutput(role, loggedEntry?.text ?? "", false, mergeKey)
  }

  return {
    appendProviderChunk,
    appendToolUpdate,
  }
}

function findUpdatedEntry(entries: TranscriptEntry[], updatedEntryId: number | undefined) {
  if (updatedEntryId === undefined) {
    return undefined
  }
  return entries.find((entry) => entry.id === updatedEntryId)
}
