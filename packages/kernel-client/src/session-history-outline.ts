import type {
  SessionHistoryEntry,
  SessionHistoryOutline,
  SessionHistoryOutlineBlob,
  SessionHistoryOutlineCursor,
  SessionHistoryOutlineTurn,
  SessionHistoryPageEntry,
  TranscriptEntry,
} from "./kernel-types.js"

export type SessionHistoryCursorSelection = {
  readonly agentId: string
  readonly cursor: SessionHistoryOutlineCursor
} | null

export type SessionHistoryTranscriptRole = Exclude<TranscriptEntry["role"], "turn_toggle">

export type SessionHistoryOutlineTurnItem<
  TEntry extends SessionHistoryPageEntry = SessionHistoryPageEntry,
  TBlob extends SessionHistoryOutlineBlob = SessionHistoryOutlineBlob,
> =
  | {
    readonly kind: "entry"
    readonly sequence: number
    readonly entry: TEntry
  }
  | {
    readonly kind: "blob"
    readonly sequence: number
    readonly blob: TBlob
  }

export type SessionHistoryOutlineTurnLike = {
  readonly user_prompt: SessionHistoryPageEntry
}

export type SessionHistoryOutlineTurnItemsLike<
  TEntry extends SessionHistoryPageEntry = SessionHistoryPageEntry,
  TBlob extends SessionHistoryOutlineBlob = SessionHistoryOutlineBlob,
> = {
  readonly entries: readonly TEntry[]
  readonly summary?: TEntry | null | undefined
  readonly blobs: readonly TBlob[]
}

export type SessionHistoryOutlineTurnCompletionLike = {
  readonly completed_at_ms?: number | null | undefined
}

export function orderedSessionHistoryOutlineTurns<TTurn extends SessionHistoryOutlineTurnLike>(
  turns: readonly TTurn[],
): TTurn[] {
  return [...turns].sort((left, right) =>
    sessionHistoryPageEntryIndex(left.user_prompt) - sessionHistoryPageEntryIndex(right.user_prompt))
}

export function orderedSessionHistoryOutlineItems<
  TEntry extends SessionHistoryPageEntry,
  TBlob extends SessionHistoryOutlineBlob,
>(
  turn: SessionHistoryOutlineTurnItemsLike<TEntry, TBlob>,
): SessionHistoryOutlineTurnItem<TEntry, TBlob>[] {
  return [
    ...turn.entries.map((entry): SessionHistoryOutlineTurnItem<TEntry, TBlob> => ({
      kind: "entry",
      sequence: sessionHistoryPageEntryIndex(entry),
      entry,
    })),
    ...turn.blobs.map((blob): SessionHistoryOutlineTurnItem<TEntry, TBlob> => ({
      kind: "blob",
      sequence: sessionHistoryOutlineBlobSequenceStart(blob),
      blob,
    })),
    ...(turn.summary ? [{
      kind: "entry" as const,
      sequence: sessionHistoryPageEntryIndex(turn.summary),
      entry: turn.summary,
    }] : []),
  ].sort((left, right) => left.sequence - right.sequence)
}

export function sessionHistoryPageEntryIndex(pageEntry: Pick<SessionHistoryPageEntry, "entry_index">): number {
  return Number.isFinite(pageEntry.entry_index) ? pageEntry.entry_index : Number.MAX_SAFE_INTEGER
}

export function sessionHistoryOutlineBlobSequenceStart(blob: Pick<SessionHistoryOutlineBlob, "sequence_start">): number {
  return Number.isFinite(blob.sequence_start) ? blob.sequence_start : Number.MAX_SAFE_INTEGER
}

export function sessionHistoryOutlineTurnDisplayId(
  turn: Pick<SessionHistoryOutlineTurn, "user_prompt">,
  turnIndex: number,
): number {
  const promptIndex = sessionHistoryPageEntryIndex(turn.user_prompt)
  return Number.isFinite(promptIndex) && promptIndex < Number.MAX_SAFE_INTEGER
    ? promptIndex + 1
    : turnIndex + 1
}

export function sessionHistoryOutlineTurnCompletedAtMs(
  turn: SessionHistoryOutlineTurnCompletionLike,
): number | null | undefined {
  if (!Object.prototype.hasOwnProperty.call(turn, "completed_at_ms")) {
    return undefined
  }
  return turn.completed_at_ms ?? null
}

export function sessionHistoryCursorForVisibleAgent(
  outline: SessionHistoryOutline,
  visibleAgentId: string | null,
): SessionHistoryCursorSelection {
  if (!visibleAgentId) {
    return null
  }
  const cursor = outline.agents.find((agent) => agent.agent_id === visibleAgentId)?.next_cursor
  return cursor ? { agentId: visibleAgentId, cursor } : null
}

export function sessionHistoryEntryKindTranscriptRole(
  kind: SessionHistoryEntry["kind"],
): SessionHistoryTranscriptRole {
  switch (kind) {
    case "user_prompt":
      return "user"
    case "provider_output":
      return "assistant"
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
  }
}
