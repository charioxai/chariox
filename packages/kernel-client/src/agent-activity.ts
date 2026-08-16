import { promptOriginIsExternal } from "./prompt-origin.js"
import {
  kernelRecordTranscriptMetadata,
  presentTranscriptPromptMetadataFields,
  type PresentTranscriptPromptMetadata,
} from "./transcript-entry-state.js"
import {
  externalProviderObservedExactIdentityConflicts,
  type ExternalProviderObservedIdentityFields,
} from "./external-provider-observation.js"

export type AgentRuntimeActivityBusyInput = {
  readonly busy?: boolean | null
  readonly status?: string | null
  readonly prompt_status?: string | null
  readonly active_prompt_count?: number | null
  readonly queued_prompt_count?: number | null
  readonly active_turn?: unknown | null
  readonly error?: boolean | null
}

export type LegacyAgentProcessingInput = {
  readonly is_processing?: boolean | null
  readonly state?: string | null
}

export type AgentRuntimeActivityStatus = "idle" | "working" | "error"
export type AgentRuntimePromptStatus =
  | "none"
  | "queued"
  | "dispatching"
  | "running"
  | "cancelling"
  | "settling"

export type AgentRuntimeActivityProjection = {
  readonly status: AgentRuntimeActivityStatus | null
  readonly promptStatus: AgentRuntimePromptStatus
  readonly busy: boolean
  readonly activeTurn: Record<string, unknown> | null
  readonly activeTurnPromptId?: string
  readonly activeTurnProviderRunId?: string
  readonly activeTurnSourceAttachmentId?: string | null
  readonly activeTurnPromptOrigin?: string
  readonly activeTurnPromptSource?: string
  readonly activeTurnExternalProvider?: string
  readonly activeTurnExternalProviderSessionId?: string
  readonly activeTurnExternalProviderTurnId?: string
  readonly activeTurnStatus?: AgentRuntimePromptStatus
  readonly activeTurnPhase?: string
  readonly activeTurnStartedAtMs?: number
  readonly lastCompletedTurn?: AgentRuntimeCompletedTurnActionProjection | null
  readonly activePromptCount: number
  readonly activePromptCountExplicit: boolean
  readonly queuedPromptCount: number
  readonly queuedPromptCountExplicit: boolean
  readonly error: boolean
  readonly unreadIdleOutput: boolean
}

export type AgentRuntimeCompletedTurnActionProjection = {
  readonly turnId: string
  readonly promptId: string
  readonly providerRunId: string
  readonly agentId: string
  readonly sourceAttachmentId?: string | null
  readonly promptOrigin?: string
  readonly externalProvider?: string
  readonly externalProviderSessionId?: string
  readonly externalProviderTurnId?: string
  readonly completedAtMs: number
  readonly durationMs: number | null
  readonly changedPaths: readonly string[]
  readonly undoAvailable: boolean
  readonly undoUnavailableReason: string | null
}

type AgentRuntimeExternalIdentityProjection = Pick<
  AgentRuntimeCompletedTurnActionProjection,
  "externalProvider" | "externalProviderSessionId" | "externalProviderTurnId"
>

const TURN_ALREADY_UNDONE_REASON = "turn already undone"

export function agentRuntimeActivityIsBusy(
  activity: AgentRuntimeActivityBusyInput | null | undefined,
): boolean {
  return projectAgentRuntimeActivity(activity).busy
}

export function agentLegacyProcessingStateIsBusy(
  agent: LegacyAgentProcessingInput | null | undefined,
): boolean {
  return Boolean(agent?.is_processing === true || agent?.state === "Working")
}

export function agentRuntimeActivityHasTurnWork(activity: unknown): boolean {
  return agentRuntimeActivityProjectionHasTurnWork(projectAgentRuntimeActivity(activity))
}

export function agentRuntimeActivityProjectionHasTurnWork(
  projection: AgentRuntimeActivityProjection,
): boolean {
  if (
    projection.activeTurnPromptId
    || projection.activeTurnProviderRunId
    || projection.activeTurnExternalProviderSessionId
    || projection.activeTurnExternalProviderTurnId
    || projection.activeTurnStatus
    || projection.activeTurnPhase
    || projection.activeTurnStartedAtMs !== undefined
  ) {
    return true
  }
  if (agentRuntimePromptStatusIsActivePrompt(projection.promptStatus)) {
    return true
  }
  if (projection.activePromptCount > 0) {
    return true
  }
  if (projection.promptStatus === "queued" || projection.queuedPromptCount > 0) {
    return false
  }
  return projection.status === "working" || projection.busy
}

export function agentRuntimeActivityProjectionHasExternalActiveTurn(
  projection: AgentRuntimeActivityProjection,
): boolean {
  return promptOriginIsExternal(projection.activeTurnPromptOrigin)
}

export function agentRuntimeActivityResolvedStatus(
  activity: unknown,
): AgentRuntimeActivityStatus {
  return agentRuntimeActivityProjectionResolvedStatus(projectAgentRuntimeActivity(activity))
}

export function agentRuntimeActivityProjectionResolvedStatus(
  projection: AgentRuntimeActivityProjection,
): AgentRuntimeActivityStatus {
  if (projection.error || projection.status === "error") {
    return "error"
  }
  return projection.busy ? "working" : "idle"
}

export function projectAgentRuntimeActivity(
  value: unknown,
  options: { readonly previousError?: boolean } = {},
): AgentRuntimeActivityProjection {
  const activityRecord = readAgentRuntimeActivityRecord(value) ?? {}
  const status = normalizeAgentRuntimeActivityProjectionStatus(
    readStringField(activityRecord, "status") ?? readStringField(value, "status"),
  )
  const promptStatus = normalizeAgentRuntimePromptProjectionStatus(
    readStringField(activityRecord, "prompt_status") ?? readStringField(value, "prompt_status"),
  ) ?? "none"
  const activeTurn = readRecordField(activityRecord, "active_turn") ?? readRecordField(value, "active_turn")
  const activeTurnBusy = agentRuntimeActiveTurnIsBusy(activeTurn)
  const liveActiveTurn = activeTurnBusy ? activeTurn : null
  const rawBusy = readBooleanField(activityRecord, "busy") === true
    || readBooleanField(value, "busy") === true
    || status === "working"
    || agentRuntimePromptStatusIsActivePrompt(promptStatus)
    || activeTurnBusy
  const projectedActivePromptCount = readExplicitNonNegativeIntegerField(
    activityRecord,
    value,
    "active_prompt_count",
  )
  const activePromptCount = projectedActivePromptCount.value
    ?? (agentRuntimePromptStatusIsActivePrompt(promptStatus) || activeTurnBusy ? 1 : 0)
  const projectedQueuedPromptCount = readExplicitNonNegativeIntegerField(
    activityRecord,
    value,
    "queued_prompt_count",
  )
  const queuedPromptCount = projectedQueuedPromptCount.value
    ?? (agentRuntimePromptStatusIsQueued(promptStatus) ? 1 : 0)
  const error = readBooleanField(activityRecord, "error")
    ?? readBooleanField(value, "error")
    ?? (status ? status === "error" : options.previousError ?? false)
  const unreadIdleOutput = readBooleanField(activityRecord, "unread_idle_output")
    ?? readBooleanField(value, "unread_idle_output")
    ?? false
  const activeTurnIdentity = projectAgentRuntimeActiveTurnIdentity(liveActiveTurn)
  const lastCompletedTurn = readAgentRuntimeCompletedTurn(activityRecord)
    ?? readAgentRuntimeCompletedTurn(value)
  const projection: AgentRuntimeActivityProjection = {
    status,
    promptStatus,
    busy: rawBusy,
    activeTurn,
    ...activeTurnIdentity,
    ...(lastCompletedTurn ? { lastCompletedTurn } : {}),
    activePromptCount,
    activePromptCountExplicit: projectedActivePromptCount.explicit,
    queuedPromptCount,
    queuedPromptCountExplicit: projectedQueuedPromptCount.explicit,
    error,
    unreadIdleOutput,
  }
  return {
    ...projection,
    busy: agentRuntimeActivityProjectionHasTurnWork(projection),
  }
}

export function readAgentRuntimeActivityRecord(value: unknown): Record<string, unknown> | null {
  return readRecordField(value, "activity") ?? readRecord(value)
}

export function normalizeAgentRuntimeActivityStatus(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase()
  return normalized || null
}

export function normalizeAgentRuntimePromptStatus(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase()
  return normalized || null
}

export function normalizeAgentRuntimeProviderId(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase()
  return normalized || null
}

export function normalizeAgentRuntimeActivityProjectionStatus(
  value: string | null | undefined,
): AgentRuntimeActivityStatus | null {
  switch (normalizeAgentRuntimeActivityStatus(value)) {
    case "working":
      return "working"
    case "error":
      return "error"
    case "focused":
    case "idle":
      return "idle"
    default:
      return null
  }
}

export function normalizeAgentRuntimePromptProjectionStatus(
  value: string | null | undefined,
): AgentRuntimePromptStatus | null {
  switch (normalizeAgentRuntimePromptStatus(value)) {
    case "queued":
      return "queued"
    case "dispatching":
      return "dispatching"
    case "running":
      return "running"
    case "cancelling":
      return "cancelling"
    case "settling":
      return "settling"
    case "none":
    case "completed":
    case "cancelled":
    case "canceled":
    case "failed":
      return "none"
    default:
      return null
  }
}

export function agentRuntimePromptStatusIsActive(value: string | null): boolean {
  return value === "queued"
    || value === "dispatching"
    || value === "running"
    || value === "cancelling"
    || value === "settling"
}

export function agentRuntimePromptStatusIsQueued(value: string | null): boolean {
  return value === "queued"
}

export function agentRuntimePromptStatusIsActivePrompt(value: string | null): boolean {
  return value === "running"
    || value === "dispatching"
    || value === "cancelling"
    || value === "settling"
}

export function agentRuntimeActiveTurnIsBusy(activeTurn: unknown): boolean {
  if (!activeTurn || typeof activeTurn !== "object" || Array.isArray(activeTurn)) {
    return false
  }
  const rawStatus = (activeTurn as { readonly status?: unknown }).status
  const status = normalizeAgentRuntimePromptStatus(typeof rawStatus === "string" ? rawStatus : null)
  return status === null || agentRuntimePromptStatusIsActivePrompt(status)
}

export function readAgentRuntimeCompletedTurn(
  value: unknown,
): AgentRuntimeCompletedTurnActionProjection | null {
  const turn = readRecordField(value, "last_completed_turn")
  if (!turn) {
    return null
  }
  const turnId = readNonBlankStringField(turn, "turn_id")
  const promptId = readNonBlankStringField(turn, "prompt_id")
  const providerRunId = readNonBlankStringField(turn, "provider_run_id")
  const agentId = readNonBlankStringField(turn, "agent_id")
  const completedAtMs = readNumberField(turn, "completed_at_ms")
  if (!turnId || !promptId || !providerRunId || !agentId || completedAtMs === null) {
    return null
  }
  return {
    turnId,
    promptId,
    providerRunId,
    agentId,
    ...projectAgentRuntimeCompletedTurnSourceAttachment(turn),
    ...projectAgentRuntimeCompletedTurnOwnership(turn),
    completedAtMs,
    durationMs: readNumberField(turn, "duration_ms"),
    changedPaths: readStringArrayField(turn, "changed_paths"),
    undoAvailable: readBooleanField(turn, "undo_available") === true,
    undoUnavailableReason: readStringField(turn, "undo_unavailable_reason"),
  }
}

function projectAgentRuntimeCompletedTurnSourceAttachment(
  turn: Record<string, unknown>,
): Pick<AgentRuntimeCompletedTurnActionProjection, "sourceAttachmentId"> {
  const sourceAttachmentId = projectAgentRuntimeTurnPromptMetadata(turn).sourceAttachmentId
  return sourceAttachmentId !== undefined ? { sourceAttachmentId } : {}
}

function projectAgentRuntimeCompletedTurnOwnership(
  turn: Record<string, unknown>,
): Pick<AgentRuntimeCompletedTurnActionProjection,
  | "promptOrigin"
  | "externalProvider"
  | "externalProviderSessionId"
  | "externalProviderTurnId"
> {
  const metadata = projectAgentRuntimeTurnPromptMetadata(turn)
  const externalIdentity = projectAgentRuntimeExternalIdentity(metadata)
  return {
    ...(metadata.promptOrigin ? { promptOrigin: metadata.promptOrigin } : {}),
    ...externalIdentity,
  }
}

export function agentRuntimeCompletedTurnAlreadyUndone(
  turn: AgentRuntimeCompletedTurnActionProjection | null | undefined,
): boolean {
  return turn?.undoAvailable === false && turn.undoUnavailableReason === TURN_ALREADY_UNDONE_REASON
}

export function agentRuntimeCompletedTurnMatches(
  current: AgentRuntimeCompletedTurnActionProjection,
  incoming: AgentRuntimeCompletedTurnActionProjection,
): boolean {
  return current.turnId === incoming.turnId
    && !externalProviderObservedExactIdentityConflicts(
      completedTurnExternalIdentityFields(current),
      completedTurnExternalIdentityFields(incoming),
    )
}

export function reconcileAgentRuntimeLastCompletedTurn(
  current: AgentRuntimeCompletedTurnActionProjection | null | undefined,
  incoming: AgentRuntimeCompletedTurnActionProjection | null | undefined,
): AgentRuntimeCompletedTurnActionProjection | null {
  const existing = current ?? null
  if (
    existing
    && incoming
    && agentRuntimeCompletedTurnAlreadyUndone(existing)
    && agentRuntimeCompletedTurnMatches(existing, incoming)
  ) {
    return existing
  }
  if (existing && incoming && agentRuntimeCompletedTurnMatches(existing, incoming)) {
    return mergeAgentRuntimeCompletedTurnOwnership(existing, incoming)
  }
  return incoming ?? existing
}

function mergeAgentRuntimeCompletedTurnOwnership(
  current: AgentRuntimeCompletedTurnActionProjection,
  incoming: AgentRuntimeCompletedTurnActionProjection,
): AgentRuntimeCompletedTurnActionProjection {
  const preservesOwnership =
    (incoming.sourceAttachmentId === undefined && current.sourceAttachmentId !== undefined)
    || (incoming.promptOrigin === undefined && current.promptOrigin !== undefined)
    || (incoming.externalProvider === undefined && current.externalProvider !== undefined)
    || (incoming.externalProviderSessionId === undefined && current.externalProviderSessionId !== undefined)
    || (incoming.externalProviderTurnId === undefined && current.externalProviderTurnId !== undefined)
  if (!preservesOwnership) {
    return incoming
  }
  const promptOrigin = incoming.promptOrigin ?? current.promptOrigin
  const sourceAttachmentId = incoming.sourceAttachmentId ?? current.sourceAttachmentId
  const externalProvider = incoming.externalProvider ?? current.externalProvider
  const externalProviderSessionId = incoming.externalProviderSessionId ?? current.externalProviderSessionId
  const externalProviderTurnId = incoming.externalProviderTurnId ?? current.externalProviderTurnId
  return {
    ...incoming,
    ...(sourceAttachmentId !== undefined ? { sourceAttachmentId } : {}),
    ...(promptOrigin !== undefined ? { promptOrigin } : {}),
    ...(externalProvider !== undefined ? { externalProvider } : {}),
    ...(externalProviderSessionId !== undefined ? { externalProviderSessionId } : {}),
    ...(externalProviderTurnId !== undefined ? { externalProviderTurnId } : {}),
  }
}

function completedTurnExternalIdentityFields(
  turn: AgentRuntimeCompletedTurnActionProjection,
): ExternalProviderObservedIdentityFields {
  return {
    ...(turn.externalProvider !== undefined ? { externalProvider: turn.externalProvider } : {}),
    ...(turn.externalProviderSessionId !== undefined
      ? { externalProviderSessionId: turn.externalProviderSessionId }
      : {}),
    ...(turn.externalProviderTurnId !== undefined ? { externalProviderTurnId: turn.externalProviderTurnId } : {}),
  }
}

export function agentRuntimeCompletedTurnIsNewer(
  current: AgentRuntimeCompletedTurnActionProjection | null | undefined,
  incoming: AgentRuntimeCompletedTurnActionProjection,
): boolean {
  return !current || incoming.completedAtMs > current.completedAtMs
}

export function agentRuntimeCompletedTurnCanRestoreUndoAvailability(
  current: AgentRuntimeCompletedTurnActionProjection,
  incoming: AgentRuntimeCompletedTurnActionProjection,
): boolean {
  if (!agentRuntimeCompletedTurnMatches(current, incoming)) {
    return false
  }
  if (agentRuntimeCompletedTurnAlreadyUndone(current)) {
    return false
  }
  return incoming.undoAvailable === true && current.undoAvailable !== true
}

function projectAgentRuntimeActiveTurnIdentity(activeTurn: Record<string, unknown> | null) {
  if (!activeTurn) {
    return {}
  }
  const metadata = projectAgentRuntimeTurnPromptMetadata(activeTurn)
  const activeTurnPromptId = readStringField(activeTurn, "prompt_id") ?? undefined
  const activeTurnProviderRunId = readStringField(activeTurn, "provider_run_id") ?? undefined
  const activeTurnSourceAttachmentId = metadata.sourceAttachmentId
  const activeTurnPromptOrigin = metadata.promptOrigin
  const activeTurnPromptSource = readNullableStringField(activeTurn, "prompt_source") ?? undefined
  const activeTurnExternalIdentity = projectAgentRuntimeExternalIdentity(metadata)
  const activeTurnStatus = normalizeAgentRuntimePromptProjectionStatus(readStringField(activeTurn, "status")) ?? undefined
  const activeTurnPhase = readStringField(activeTurn, "phase") ?? undefined
  const activeTurnStartedAtMs = readNumberField(activeTurn, "started_at_ms") ?? undefined
  return {
    ...(activeTurnPromptId ? { activeTurnPromptId } : {}),
    ...(activeTurnProviderRunId ? { activeTurnProviderRunId } : {}),
    ...(activeTurnSourceAttachmentId !== undefined ? { activeTurnSourceAttachmentId } : {}),
    ...(activeTurnPromptOrigin ? { activeTurnPromptOrigin } : {}),
    ...(activeTurnPromptSource ? { activeTurnPromptSource } : {}),
    ...(activeTurnExternalIdentity.externalProvider
      ? { activeTurnExternalProvider: activeTurnExternalIdentity.externalProvider }
      : {}),
    ...(activeTurnExternalIdentity.externalProviderSessionId
      ? { activeTurnExternalProviderSessionId: activeTurnExternalIdentity.externalProviderSessionId }
      : {}),
    ...(activeTurnExternalIdentity.externalProviderTurnId
      ? { activeTurnExternalProviderTurnId: activeTurnExternalIdentity.externalProviderTurnId }
      : {}),
    ...(activeTurnStatus ? { activeTurnStatus } : {}),
    ...(activeTurnPhase ? { activeTurnPhase } : {}),
    ...(activeTurnStartedAtMs !== undefined ? { activeTurnStartedAtMs } : {}),
  }
}

function projectAgentRuntimeTurnPromptMetadata(
  record: Record<string, unknown>,
): PresentTranscriptPromptMetadata {
  const promptOrigin = readNullableStringField(record, "prompt_origin")
  const sourceAttachmentId = readNullableStringField(record, "source_attachment_id")
  const externalProvider = readNullableStringField(record, "external_provider")
  const externalProviderSessionId = readNullableStringField(record, "external_provider_session_id")
  const externalProviderTurnId = readNullableStringField(record, "external_provider_turn_id")
  return presentTranscriptPromptMetadataFields(kernelRecordTranscriptMetadata({
    ...(promptOrigin !== undefined ? { prompt_origin: promptOrigin } : {}),
    ...(sourceAttachmentId !== undefined ? { source_attachment_id: sourceAttachmentId } : {}),
    ...(externalProvider !== undefined ? { external_provider: externalProvider } : {}),
    ...(externalProviderSessionId !== undefined ? { external_provider_session_id: externalProviderSessionId } : {}),
    ...(externalProviderTurnId !== undefined ? { external_provider_turn_id: externalProviderTurnId } : {}),
  }))
}

function projectAgentRuntimeExternalIdentity(
  metadata: PresentTranscriptPromptMetadata,
): AgentRuntimeExternalIdentityProjection {
  return {
    ...(metadata.externalProvider ? { externalProvider: metadata.externalProvider } : {}),
    ...(metadata.externalProviderSessionId ? { externalProviderSessionId: metadata.externalProviderSessionId } : {}),
    ...(metadata.externalProviderTurnId ? { externalProviderTurnId: metadata.externalProviderTurnId } : {}),
  }
}

function readRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function readRecordField(value: unknown, field: string): Record<string, unknown> | null {
  const record = readRecord(value)
  return readRecord(record?.[field])
}

function readStringField(value: unknown, field: string): string | null {
  const record = readRecord(value)
  const candidate = record?.[field]
  return typeof candidate === "string" ? candidate : null
}

function readNullableStringField(value: unknown, field: string): string | null | undefined {
  const record = readRecord(value)
  if (!record || !Object.prototype.hasOwnProperty.call(record, field)) {
    return undefined
  }
  const candidate = record[field]
  return typeof candidate === "string" ? candidate : null
}

function readBooleanField(value: unknown, field: string): boolean | null {
  const record = readRecord(value)
  const candidate = record?.[field]
  return typeof candidate === "boolean" ? candidate : null
}

function readNumberField(value: unknown, field: string): number | null {
  const record = readRecord(value)
  const candidate = record?.[field]
  return typeof candidate === "number" && Number.isFinite(candidate) ? candidate : null
}

function readNonNegativeIntegerField(value: unknown, field: string): number | null {
  const record = readRecord(value)
  const candidate = record?.[field]
  return typeof candidate === "number" && Number.isInteger(candidate) && candidate >= 0
    ? candidate
    : null
}

function readExplicitNonNegativeIntegerField(
  primary: unknown,
  fallback: unknown,
  field: string,
): { readonly explicit: boolean, readonly value: number | null } {
  const primaryValue = readNonNegativeIntegerField(primary, field)
  if (primaryValue !== null) {
    return { explicit: true, value: primaryValue }
  }
  const fallbackValue = readNonNegativeIntegerField(fallback, field)
  if (fallbackValue !== null) {
    return { explicit: true, value: fallbackValue }
  }
  return { explicit: false, value: null }
}

function readNonBlankStringField(value: unknown, field: string): string | null {
  const candidate = readStringField(value, field)
  return candidate?.trim() ? candidate.trim() : null
}

function readStringArrayField(value: unknown, field: string): string[] {
  const record = readRecord(value)
  const candidate = record?.[field]
  return Array.isArray(candidate)
    ? candidate.filter((entry): entry is string => typeof entry === "string")
    : []
}
