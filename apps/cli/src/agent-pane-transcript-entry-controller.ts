import type { TranscriptEntry } from "./cli-types.js"
import {
  appendPreviewLine,
  computeCurrentTurnId,
  formatTranscriptPreview,
} from "./transcript-preview.js"
import { shouldSkipConsecutiveTranscriptEntry } from "./transcript.js"
import { trimSingleTrailingNewline } from "./transcript-text.js"

export type AgentPaneTranscriptEntryControllerDeps = {
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  visibleTranscriptAgentId: () => string | null | undefined
  visibleTranscriptEntries: () => TranscriptEntry[]
  expandedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  setAgentPanePreview: (agentId: string, text: string) => void
  updateAgentPanePreviews: (
    updater: (current: Record<string, string>) => Record<string, string>,
  ) => void
  trimLiveAgentPaneEntries: (agentId: string, entries: TranscriptEntry[]) => TranscriptEntry[]
  setAgentTranscriptEntries: (
    agentId: string,
    entries: TranscriptEntry[],
    turnIds?: readonly number[],
  ) => void
}

export function createAgentPaneTranscriptEntryController(
  deps: AgentPaneTranscriptEntryControllerDeps,
) {
  const syncVisibleTranscriptPreview = (
    agentId: string | null | undefined = deps.visibleTranscriptAgentId(),
    previewEntries: readonly TranscriptEntry[] = deps.visibleTranscriptEntries(),
  ) => {
    if (!agentId) {
      return
    }
    deps.setAgentPanePreview(agentId, formatTranscriptPreview([...previewEntries]))
  }

  const appendPreview = (agentId: string | null | undefined, line: string) => {
    if (!agentId) {
      return
    }
    const normalized = normalizePreviewLine(line)
    if (!normalized) {
      return
    }
    deps.updateAgentPanePreviews((current) => ({
      ...current,
      [agentId]: appendPreviewLine(current[agentId] ?? "", normalized),
    }))
  }

  const hasTrailingUserPrompt = (agentId: string, text: string) => {
    const lastEntry = deps.currentAgentPaneEntries(agentId).at(-1)
    return lastEntry?.role === "user"
      && trimSingleTrailingNewline(lastEntry.text) === trimSingleTrailingNewline(text)
  }

  const appendEntry = (
    agentId: string,
    entry: Omit<TranscriptEntry, "id">,
    turnIds = deps.expandedTurnIdsForAgent(agentId),
  ) => {
    const currentEntries = deps.currentAgentPaneEntries(agentId).map((item) => ({ ...item }))
    const previousEntry = currentEntries.at(-1)
    if (shouldSkipConsecutiveTranscriptEntry(previousEntry, entry)) {
      return
    }
    const nextEntry = createNextTranscriptEntry(currentEntries, entry)
    deps.setAgentTranscriptEntries(
      agentId,
      deps.trimLiveAgentPaneEntries(agentId, [...currentEntries, nextEntry]),
      turnIds,
    )
  }

  return {
    appendEntry,
    appendPreview,
    hasTrailingUserPrompt,
    syncVisibleTranscriptPreview,
  }
}

function normalizePreviewLine(line: string) {
  return line.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
}

function createNextTranscriptEntry(
  currentEntries: TranscriptEntry[],
  entry: Omit<TranscriptEntry, "id">,
) {
  const nextEntry: TranscriptEntry = {
    id: currentEntries.reduce((max, current) => Math.max(max, current.id), 0) + 1,
    ...entry,
  }
  if (nextEntry.turnId === undefined) {
    const activeTurnId = computeCurrentTurnId(currentEntries)
    if (activeTurnId !== null) {
      nextEntry.turnId = activeTurnId
    }
  }
  return nextEntry
}
