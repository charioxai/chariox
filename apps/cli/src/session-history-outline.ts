import type {
  SessionHistoryBlobContent,
  SessionHistoryOutlineAgent,
  SessionHistoryOutlineBlob,
  SessionHistoryOutlineTurn,
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
    const externalMetadata = outlineTurnExternalMetadata(turn)
    const promptEntries = hydratePageEntries([turn.user_prompt], turnId, turn.prompt_id ?? null)
    for (const entry of promptEntries) {
      entries.push(applyOutlineTurnExternalMetadata({ ...entry, id: ++nextId }, externalMetadata))
    }
    const turnItems = [
      ...turn.entries.map((entry) => ({ sequence: entry.entry_index, entry })),
      ...turn.blobs.map((blob) => ({ sequence: blob.sequence_start, blob })),
      ...(turn.summary ? [{ sequence: turn.summary.entry_index, entry: turn.summary }] : []),
    ].sort((left, right) => left.sequence - right.sequence)
    for (const item of turnItems) {
      if ("blob" in item) {
        entries.push(applyOutlineTurnExternalMetadata(
          outlineBlobEntry(item.blob, agent.agent_id, turnId, turn.prompt_id ?? null, ++nextId),
          externalMetadata,
        ))
        continue
      }
      const hydratedEntries = hydratePageEntries([item.entry], turnId, turn.prompt_id ?? null)
      for (const entry of hydratedEntries) {
        entries.push(applyOutlineTurnExternalMetadata({ ...entry, id: ++nextId }, externalMetadata))
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
  const externalMetadata = transcriptEntryExternalMetadata(placeholder)
  const hydrated = hydratePageEntries(content.entries, turnId, placeholder.promptId ?? null).map((entry) => {
    const next: TranscriptEntry = {
      ...entry,
      blobCollapsed: false,
      historyBlobLoaded: true,
    }
    if (placeholder.historyBlobId) {
      next.historyBlobSourceId = placeholder.historyBlobId
    }
    if (placeholder.historyBlobAgentId) {
      next.historyBlobSourceAgentId = placeholder.historyBlobAgentId
    }
    return applyOutlineTurnExternalMetadata(next, externalMetadata)
  })
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

type OutlineTurnExternalMetadata = {
  externalProvider: string
  externalProviderSessionId: string
  externalProviderTurnId: string
}

function outlineTurnExternalMetadata(
  turn: SessionHistoryOutlineTurn,
): OutlineTurnExternalMetadata | null {
  const externalProvider = nonBlankString(turn.external_provider)
  const externalProviderSessionId = nonBlankString(turn.external_provider_session_id)
  const externalProviderTurnId = nonBlankString(turn.external_provider_turn_id)
  if (!externalProvider || !externalProviderSessionId || !externalProviderTurnId) {
    return null
  }
  return {
    externalProvider,
    externalProviderSessionId,
    externalProviderTurnId,
  }
}

function applyOutlineTurnExternalMetadata(
  entry: TranscriptEntry,
  metadata: OutlineTurnExternalMetadata | null,
): TranscriptEntry {
  if (!metadata) {
    return entry
  }
  return {
    ...entry,
    source: entry.source ?? "external_provider_observed",
    externalProvider: entry.externalProvider ?? metadata.externalProvider,
    externalProviderSessionId: entry.externalProviderSessionId ?? metadata.externalProviderSessionId,
    externalProviderTurnId: entry.externalProviderTurnId ?? metadata.externalProviderTurnId,
  }
}

function transcriptEntryExternalMetadata(
  entry: TranscriptEntry,
): OutlineTurnExternalMetadata | null {
  const externalProvider = nonBlankString(entry.externalProvider)
  const externalProviderSessionId = nonBlankString(entry.externalProviderSessionId)
  const externalProviderTurnId = nonBlankString(entry.externalProviderTurnId)
  if (!externalProvider || !externalProviderSessionId || !externalProviderTurnId) {
    return null
  }
  return {
    externalProvider,
    externalProviderSessionId,
    externalProviderTurnId,
  }
}

function nonBlankString(value: string | null | undefined): string | null {
  return value?.trim() ? value.trim() : null
}

function hydratePageEntries(
  pageEntries: SessionHistoryPageEntry[],
  turnId?: number,
  promptId?: string | null,
): TranscriptEntry[] {
  const hydrateOptions = promptId === undefined ? {} : { promptId }
  return hydrateTranscriptEntries(pageEntries, hydrateOptions).map((entry) => ({
    ...entry,
    ...(turnId !== undefined ? { turnId } : {}),
  }))
}

function outlineBlobEntry(
  blob: SessionHistoryOutlineBlob,
  agentId: string,
  turnId: number,
  promptId: string | null,
  id: number,
): TranscriptEntry {
  return {
    id,
    role: roleForHistoryKind(blob.kind),
    text: "",
    sourceText: "",
    turnId,
    promptId,
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
