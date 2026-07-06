import type { TranscriptEntry } from "./cli-types.js"
import {
  appendTranscriptPreviewLine as appendPreviewLine,
  formatTranscriptPreview,
} from "@arroba/kernel-client/session-history-preview"
import {
  createNextTranscriptEntry,
  shouldSkipConsecutiveTranscriptEntry,
  transcriptHasTrailingUserPrompt,
} from "@arroba/kernel-client/transcript-entry-state"

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

  const hasTrailingUserPrompt = (agentId: string, text: string, promptId?: string | null) => {
    return transcriptHasTrailingUserPrompt(deps.currentAgentPaneEntries(agentId), text, promptId)
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
    const nextEntry = createNextTranscriptEntry<TranscriptEntry, Omit<TranscriptEntry, "id">>(currentEntries, entry)
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
