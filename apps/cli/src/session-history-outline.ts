import type {
  SessionHistoryBlobContent,
  SessionHistoryOutlineAgent,
  SessionHistoryOutlineBlob,
  SessionHistoryPageEntry,
  TranscriptEntry,
} from "./cli-types.js"
import { applyTranscriptDisplayState } from "./transcript-display.js"
import { hydrateTranscriptEntries } from "./transcript-history.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

export function hydrateOutlineAgentEntries(agent: SessionHistoryOutlineAgent): TranscriptEntry[] {
  const entries: TranscriptEntry[] = []
  let nextId = 0

  agent.turns.forEach((turn, turnIndex) => {
    const turnId = turnIndex + 1
    const promptEntries = hydratePageEntries([turn.user_prompt], turnId)
    for (const entry of promptEntries) {
      entries.push({ ...entry, id: ++nextId })
    }
    const turnItems = [
      ...turn.entries.map((entry) => ({ sequence: entry.entry_index, entry })),
      ...turn.blobs.map((blob) => ({ sequence: blob.sequence_start, blob })),
      ...(turn.summary ? [{ sequence: turn.summary.entry_index, entry: turn.summary }] : []),
    ].sort((left, right) => left.sequence - right.sequence)
    for (const item of turnItems) {
      if ("blob" in item) {
        entries.push(outlineBlobEntry(item.blob, agent.agent_id, turnId, ++nextId))
        continue
      }
      const hydratedEntries = hydratePageEntries([item.entry], turnId)
      for (const entry of hydratedEntries) {
        entries.push({ ...entry, id: ++nextId })
      }
    }
  })

  return applyTranscriptDisplayState(entries, [])
}

export function replaceHistoryBlobPlaceholder(
  entries: TranscriptEntry[],
  entryId: number,
  content: SessionHistoryBlobContent,
  expandedTurnIds: readonly number[],
): TranscriptEntry[] {
  const placeholder = entries.find((entry) => entry.id === entryId)
  if (!placeholder?.historyBlobId) {
    return entries
  }
  const turnId = placeholder.turnId
  const hydrated = hydratePageEntries(content.entries, turnId).map((entry) => ({
    ...entry,
    blobCollapsed: false,
    historyBlobLoaded: true,
  }))
  const replaced = entries.flatMap((entry) => entry.id === entryId ? hydrated : [entry])
  return applyTranscriptDisplayState(reindexTranscriptEntries(replaced, 0), expandedTurnIds)
}

export function markHistoryBlobLoading(
  entries: TranscriptEntry[],
  entryId: number,
  loading: boolean,
  error?: string | null,
): TranscriptEntry[] {
  return entries.map((entry) => {
    if (entry.id !== entryId || !entry.historyBlobId) {
      return entry
    }
    const next: TranscriptEntry = {
      ...entry,
      historyBlobLoading: loading,
    }
    if (loading) {
      next.blobSummary = "loading..."
      delete next.historyBlobError
      return next
    }
    if (error) {
      next.historyBlobError = error
      next.blobSummary = `failed: ${error}`
      return next
    }
    delete next.historyBlobError
    return next
  })
}

function hydratePageEntries(pageEntries: SessionHistoryPageEntry[], turnId?: number): TranscriptEntry[] {
  return hydrateTranscriptEntries(pageEntries).map((entry) => ({
    ...entry,
    ...(turnId !== undefined ? { turnId } : {}),
  }))
}

function outlineBlobEntry(
  blob: SessionHistoryOutlineBlob,
  agentId: string,
  turnId: number,
  id: number,
): TranscriptEntry {
  return {
    id,
    role: roleForHistoryKind(blob.kind),
    text: "",
    sourceText: "",
    turnId,
    blobCollapsible: true,
    blobCollapsed: true,
    blobTitle: blob.title,
    blobSummary: blob.summary,
    historyBlobId: blob.blob_id,
    historyBlobAgentId: agentId,
    historyBlobLoaded: false,
    historyEntryIndex: blob.sequence_start,
    historyFragmentStart: 0,
    historyFragmentEnd: blob.total_chars,
    historyTotalChars: blob.total_chars,
  }
}

function roleForHistoryKind(kind: SessionHistoryOutlineBlob["kind"]): TranscriptEntry["role"] {
  switch (kind) {
    case "provider_reasoning":
      return "reasoning"
    case "provider_tool":
      return "tool"
    case "provider_error":
      return "error"
    case "provider_status":
      return "status"
    case "notice":
      return "notice"
    case "provider_output":
      return "assistant"
    default:
      return "tool"
  }
}
