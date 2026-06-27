import type {
  SessionHistoryBlobContent,
  SessionHistoryCursorState,
  SessionHistoryOutline,
  SessionHistoryOutlineAgent,
  SessionHistoryOutlineBlob,
  SessionHistoryOutlineTurn,
  SessionHistoryPageEntry,
  TranscriptEntry,
} from "./cli-types.js"
import { EXTERNAL_PROVIDER_OBSERVED_SOURCE } from "@arroba/kernel-client/external-provider-observation"
import { promptOriginFromRecord, promptOriginIsExternal } from "@arroba/kernel-client/prompt-origin"
import { applyTranscriptDisplayState } from "./transcript-display.js"
import { hydrateTranscriptEntries } from "./transcript-history.js"
import { reindexTranscriptEntries } from "./transcript-text.js"

export function hydrateOutlineAgentEntries(agent: SessionHistoryOutlineAgent): TranscriptEntry[] {
  const entries: TranscriptEntry[] = []
  let nextId = 0
  let activeTurnId: number | null = null

  orderedOutlineTurns(agent.turns).forEach((turn, turnIndex) => {
    const turnId = outlineTurnDisplayId(turn, turnIndex)
    const completedAtMs = outlineTurnCompletedAtMs(turn)
    if (completedAtMs === null) {
      activeTurnId = turnId
    }
    const externalMetadata = outlineTurnExternalMetadata(turn)
    const promptEntries = hydratePageEntries([turn.user_prompt], turnId, turn.prompt_id ?? null)
    for (const entry of promptEntries) {
      entries.push(applyOutlineTurnExternalMetadata(
        applyOutlineTurnLifecycleMetadata({ ...entry, id: ++nextId }, completedAtMs),
        externalMetadata,
      ))
    }
    for (const item of orderedOutlineItems(turn)) {
      if (item.kind === "blob") {
        entries.push(applyOutlineTurnExternalMetadata(
          applyOutlineTurnLifecycleMetadata(
            outlineBlobEntry(item.blob, agent.agent_id, turnId, turn.prompt_id ?? null, ++nextId),
            completedAtMs,
          ),
          externalMetadata,
        ))
        continue
      }
      const hydratedEntries = hydratePageEntries([item.entry], turnId, turn.prompt_id ?? null)
      for (const entry of hydratedEntries) {
        entries.push(applyOutlineTurnExternalMetadata(
          applyOutlineTurnLifecycleMetadata({ ...entry, id: ++nextId }, completedAtMs),
          externalMetadata,
        ))
      }
    }
  })

  return applyTranscriptDisplayState(entries, [], activeTurnId)
}

type OutlineTurnItem =
  | {
    kind: "entry"
    sequence: number
    entry: SessionHistoryPageEntry
  }
  | {
    kind: "blob"
    sequence: number
    blob: SessionHistoryOutlineBlob
  }

function orderedOutlineTurns(
  turns: readonly SessionHistoryOutlineTurn[],
): readonly SessionHistoryOutlineTurn[] {
  return [...turns].sort((left, right) =>
    historyPageEntryIndex(left.user_prompt) - historyPageEntryIndex(right.user_prompt))
}

function orderedOutlineItems(turn: SessionHistoryOutlineTurn): readonly OutlineTurnItem[] {
  return [
    ...turn.entries.map((entry): OutlineTurnItem => ({
      kind: "entry",
      sequence: historyPageEntryIndex(entry),
      entry,
    })),
    ...turn.blobs.map((blob): OutlineTurnItem => ({
      kind: "blob",
      sequence: historyBlobSequenceStart(blob),
      blob,
    })),
    ...(turn.summary ? [{
      kind: "entry" as const,
      sequence: historyPageEntryIndex(turn.summary),
      entry: turn.summary,
    }] : []),
  ].sort((left, right) => left.sequence - right.sequence)
}

function historyPageEntryIndex(pageEntry: SessionHistoryPageEntry): number {
  return Number.isFinite(pageEntry.entry_index) ? pageEntry.entry_index : Number.MAX_SAFE_INTEGER
}

function historyBlobSequenceStart(blob: SessionHistoryOutlineBlob): number {
  return Number.isFinite(blob.sequence_start) ? blob.sequence_start : Number.MAX_SAFE_INTEGER
}

function outlineTurnDisplayId(
  turn: SessionHistoryOutlineTurn,
  turnIndex: number,
): number {
  const promptIndex = historyPageEntryIndex(turn.user_prompt)
  return Number.isFinite(promptIndex) && promptIndex < Number.MAX_SAFE_INTEGER
    ? promptIndex + 1
    : turnIndex + 1
}

export function historyCursorStateForVisibleAgent(
  outline: SessionHistoryOutline,
  visibleAgentId: string | null,
): SessionHistoryCursorState {
  if (!visibleAgentId) {
    return null
  }
  const cursor = outline.agents.find((agent) => agent.agent_id === visibleAgentId)?.next_cursor
  return cursor ? { agentId: visibleAgentId, cursor } : null
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
  const activeTurnId = placeholder.historyTurnCompletedAtMs === null
    && typeof turnId === "number"
    ? turnId
    : null
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
    return applyOutlineTurnExternalMetadata(
      applyOutlineTurnLifecycleMetadata(next, placeholder.historyTurnCompletedAtMs),
      externalMetadata,
    )
  })
  const replaced = entries.flatMap((entry) => entry.id === entryId ? hydrated : [entry])
  return applyTranscriptDisplayState(reindexTranscriptEntries(replaced, 0), expandedTurnIds, activeTurnId)
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
  source: typeof EXTERNAL_PROVIDER_OBSERVED_SOURCE
  externalProvider: string | null
  externalProviderSessionId: string | null
  externalProviderTurnId: string | null
}

function outlineTurnExternalMetadata(
  turn: SessionHistoryOutlineTurn,
): OutlineTurnExternalMetadata | null {
  const externalProvider = nonBlankString(turn.external_provider)
  const externalProviderSessionId = nonBlankString(turn.external_provider_session_id)
  const externalProviderTurnId = nonBlankString(turn.external_provider_turn_id)
  if (!promptOriginIsExternal(promptOriginFromRecord(outlineTurnPromptOriginRecord(turn)))) {
    return null
  }
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider,
    externalProviderSessionId,
    externalProviderTurnId,
  }
}

function outlineTurnPromptOriginRecord(
  turn: SessionHistoryOutlineTurn,
): {
  readonly prompt_origin?: string | null
  readonly external_provider?: string | null
  readonly external_provider_session_id?: string | null
} {
  return {
    ...(turn.prompt_origin !== undefined ? { prompt_origin: turn.prompt_origin } : {}),
    ...(turn.external_provider !== undefined ? { external_provider: turn.external_provider } : {}),
    ...(turn.external_provider_session_id !== undefined
      ? { external_provider_session_id: turn.external_provider_session_id }
      : {}),
  }
}

function applyOutlineTurnExternalMetadata(
  entry: TranscriptEntry,
  metadata: OutlineTurnExternalMetadata | null,
): TranscriptEntry {
  if (!metadata) {
    return entry
  }
  const next: TranscriptEntry = {
    ...entry,
    source: entry.source ?? metadata.source,
  }
  if (next.externalProvider === undefined && metadata.externalProvider !== null) {
    next.externalProvider = metadata.externalProvider
  }
  if (next.externalProviderSessionId === undefined && metadata.externalProviderSessionId !== null) {
    next.externalProviderSessionId = metadata.externalProviderSessionId
  }
  if (next.externalProviderTurnId === undefined && metadata.externalProviderTurnId !== null) {
    next.externalProviderTurnId = metadata.externalProviderTurnId
  }
  return next
}

function transcriptEntryExternalMetadata(
  entry: TranscriptEntry,
): OutlineTurnExternalMetadata | null {
  const externalProvider = nonBlankString(entry.externalProvider)
  const externalProviderSessionId = nonBlankString(entry.externalProviderSessionId)
  const externalProviderTurnId = nonBlankString(entry.externalProviderTurnId)
  if (entry.source !== EXTERNAL_PROVIDER_OBSERVED_SOURCE) {
    return null
  }
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider,
    externalProviderSessionId,
    externalProviderTurnId,
  }
}

function outlineTurnCompletedAtMs(turn: SessionHistoryOutlineTurn): number | null | undefined {
  if (!Object.prototype.hasOwnProperty.call(turn, "completed_at_ms")) {
    return undefined
  }
  return turn.completed_at_ms ?? null
}

function applyOutlineTurnLifecycleMetadata(
  entry: TranscriptEntry,
  completedAtMs: number | null | undefined,
): TranscriptEntry {
  if (completedAtMs === undefined) {
    return entry
  }
  return {
    ...entry,
    historyTurnCompletedAtMs: completedAtMs,
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
