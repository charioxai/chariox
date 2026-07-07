import type {
  SessionHistoryEntry,
  SessionHistoryOutline,
  SessionHistoryOutlineBlob,
  SessionHistoryOutlineCursor,
  SessionHistoryOutlineTurn,
  SessionHistoryPageEntry,
  TranscriptEntry,
} from "./kernel-types.js"
import { promptOriginFromRecord } from "./prompt-origin.js"
import { providerTranscriptRoleForKind } from "./transcript-kind-role.js"

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

export type SessionHistoryOutlineTurnLifecycle = "open" | "completed"

export type SessionHistoryOutlineTurnLifecycleLike = {
  readonly lifecycle: "open" | "completed" | string
}

export type SessionHistoryOutlineTurnKeyLike = {
  readonly prompt_id?: string | null | undefined
  readonly turn_id: string
  readonly user_prompt: Pick<SessionHistoryPageEntry, "entry_index">
}

export type SessionHistoryOutlineTurnPromptMetadata = {
  readonly promptOrigin?: string | null
  readonly sourceAttachmentId?: string | null
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

export function sessionHistoryOutlineTurnKey(
  agentId: string,
  turn: SessionHistoryOutlineTurnKeyLike,
): string {
  return `${agentId}:${turn.prompt_id ?? turn.turn_id}:${sessionHistoryPageEntryIndex(turn.user_prompt)}`
}

export function sessionHistoryOutlineTurnCompletedAtMs(
  turn: SessionHistoryOutlineTurnCompletionLike,
): number | null | undefined {
  if (!Object.prototype.hasOwnProperty.call(turn, "completed_at_ms")) {
    return undefined
  }
  return typeof turn.completed_at_ms === "number" && Number.isFinite(turn.completed_at_ms)
    ? turn.completed_at_ms
    : null
}

export function sessionHistoryOutlineTurnLifecycle(
  turn: SessionHistoryOutlineTurnLifecycleLike,
): SessionHistoryOutlineTurnLifecycle {
  return turn.lifecycle === "completed" ? "completed" : "open"
}

export function sessionHistoryOutlineTurnSourceAttachmentId(
  turn: Pick<SessionHistoryOutlineTurn, "user_prompt">,
): string | null | undefined {
  const entry = turn.user_prompt.entry
  if (!Object.prototype.hasOwnProperty.call(entry, "source_attachment_id")) {
    return undefined
  }
  return entry.source_attachment_id === undefined ? undefined : entry.source_attachment_id
}

export function sessionHistoryOutlineTurnPromptMetadata(
  turn: Pick<
    SessionHistoryOutlineTurn,
    | "prompt_id"
    | "prompt_origin"
    | "external_provider"
    | "external_provider_session_id"
    | "external_provider_turn_id"
    | "user_prompt"
  >,
): SessionHistoryOutlineTurnPromptMetadata {
  const promptOrigin = promptOriginFromRecord({
    prompt_origin: turn.prompt_origin,
  })
  const sourceAttachmentId = sessionHistoryOutlineTurnSourceAttachmentId(turn)
  return {
    ...(promptOrigin !== null || Object.prototype.hasOwnProperty.call(turn, "prompt_origin")
      ? { promptOrigin }
      : {}),
    ...(sourceAttachmentId !== undefined ? { sourceAttachmentId } : {}),
  }
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
  if (kind === "user_prompt") {
    return "user"
  }
  if (kind === "notice") {
    return "notice"
  }
  return providerTranscriptRoleForKind(kind)
}
