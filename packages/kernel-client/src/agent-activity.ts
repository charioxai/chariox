import { promptOriginFromRecord } from "./prompt-origin.js"

export type AgentRuntimeActivityBusyInput = {
  readonly busy?: boolean | null
  readonly status?: string | null
  readonly prompt_status?: string | null
  readonly active_prompt_count?: number | null
  readonly queued_prompt_count?: number | null
  readonly active_turn?: unknown | null
}

export type AgentRuntimeActivityStatus = "idle" | "working" | "error"
export type AgentRuntimePromptStatus = "none" | "queued" | "running" | "cancelling" | "settling"

export type AgentRuntimeActivityProjection = {
  readonly status: AgentRuntimeActivityStatus | null
  readonly promptStatus: AgentRuntimePromptStatus
  readonly busy: boolean
  readonly activeTurn: Record<string, unknown> | null
  readonly activeTurnPromptId?: string
  readonly activeTurnProviderRunId?: string
  readonly activeTurnPromptOrigin?: string
  readonly activeTurnExternalProvider?: string
  readonly activeTurnExternalProviderSessionId?: string
  readonly activeTurnExternalProviderTurnId?: string
  readonly activeTurnStatus?: AgentRuntimePromptStatus
  readonly activeTurnPhase?: string
  readonly activeTurnStartedAtMs?: number
  readonly lastCompletedTurn?: AgentRuntimeCompletedTurnActionProjection | null
  readonly activePromptCount: number
  readonly queuedPromptCount: number
  readonly error: boolean
}

export type AgentRuntimeCompletedTurnActionProjection = {
  readonly turnId: string
  readonly promptId: string
  readonly providerRunId: string
  readonly agentId: string
  readonly completedAtMs: number
  readonly durationMs: number | null
  readonly changedPaths: readonly string[]
  readonly undoAvailable: boolean
  readonly undoUnavailableReason: string | null
}

export function agentRuntimeActivityIsBusy(
  activity: AgentRuntimeActivityBusyInput | null | undefined,
): boolean {
  return projectAgentRuntimeActivity(activity).busy
}

export function agentRuntimeActivityHasTurnWork(activity: unknown): boolean {
  const projection = projectAgentRuntimeActivity(activity)
  if (
    projection.activeTurnPromptId
    || projection.activeTurnProviderRunId
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

export function agentRuntimeActivityResolvedStatus(
  activity: unknown,
): AgentRuntimeActivityStatus {
  const projection = projectAgentRuntimeActivity(activity)
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
    || agentRuntimePromptStatusIsActive(promptStatus)
    || activeTurnBusy
  const activePromptCount = readNonNegativeIntegerField(activityRecord, "active_prompt_count")
    ?? readNonNegativeIntegerField(value, "active_prompt_count")
    ?? (agentRuntimePromptStatusIsActivePrompt(promptStatus) || activeTurnBusy ? 1 : 0)
  const queuedPromptCount = readNonNegativeIntegerField(activityRecord, "queued_prompt_count")
    ?? readNonNegativeIntegerField(value, "queued_prompt_count")
    ?? (agentRuntimePromptStatusIsQueued(promptStatus) ? 1 : 0)
  const error = readBooleanField(activityRecord, "error")
    ?? readBooleanField(value, "error")
    ?? (status ? status === "error" : options.previousError ?? false)
  const activeTurnIdentity = projectAgentRuntimeActiveTurnIdentity(liveActiveTurn)
  const lastCompletedTurn = readAgentRuntimeCompletedTurn(activityRecord)
    ?? readAgentRuntimeCompletedTurn(value)
  return {
    status,
    promptStatus,
    busy: rawBusy || activePromptCount > 0 || queuedPromptCount > 0,
    activeTurn,
    ...activeTurnIdentity,
    ...(lastCompletedTurn ? { lastCompletedTurn } : {}),
    activePromptCount,
    queuedPromptCount,
    error,
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
    case "running":
      return "running"
    case "cancelling":
      return "cancelling"
    case "settling":
      return "settling"
    case "none":
    case "completed":
    case "cancelled":
      return "none"
    default:
      return null
  }
}

export function agentRuntimePromptStatusIsActive(value: string | null): boolean {
  return value === "queued"
    || value === "running"
    || value === "cancelling"
    || value === "settling"
}

export function agentRuntimePromptStatusIsQueued(value: string | null): boolean {
  return value === "queued"
}

export function agentRuntimePromptStatusIsActivePrompt(value: string | null): boolean {
  return value === "running"
    || value === "cancelling"
    || value === "settling"
}

export function agentRuntimeActiveTurnIsBusy(activeTurn: unknown): boolean {
  if (!activeTurn || typeof activeTurn !== "object" || Array.isArray(activeTurn)) {
    return false
  }
  const rawStatus = (activeTurn as { readonly status?: unknown }).status
  const status = normalizeAgentRuntimePromptStatus(typeof rawStatus === "string" ? rawStatus : null)
  return status === null || agentRuntimePromptStatusIsActive(status)
}

export function readAgentRuntimeCompletedTurn(
  value: unknown,
): AgentRuntimeCompletedTurnActionProjection | null {
  const turn = readRecordField(value, "last_completed_turn")
  if (!turn) {
    return null
  }
  const turnId = readStringField(turn, "turn_id")
  const promptId = readStringField(turn, "prompt_id")
  const providerRunId = readStringField(turn, "provider_run_id")
  const agentId = readStringField(turn, "agent_id")
  const completedAtMs = readNumberField(turn, "completed_at_ms")
  if (!turnId || !promptId || !providerRunId || !agentId || completedAtMs === null) {
    return null
  }
  return {
    turnId,
    promptId,
    providerRunId,
    agentId,
    completedAtMs,
    durationMs: readNumberField(turn, "duration_ms"),
    changedPaths: readStringArrayField(turn, "changed_paths"),
    undoAvailable: readBooleanField(turn, "undo_available") === true,
    undoUnavailableReason: readStringField(turn, "undo_unavailable_reason"),
  }
}

function projectAgentRuntimeActiveTurnIdentity(activeTurn: Record<string, unknown> | null) {
  if (!activeTurn) {
    return {}
  }
  const activeTurnPromptId = readStringField(activeTurn, "prompt_id") ?? undefined
  const activeTurnProviderRunId = readStringField(activeTurn, "provider_run_id") ?? undefined
  const activeTurnPromptOrigin = promptOriginFromRecord({
    prompt_origin: readStringField(activeTurn, "prompt_origin"),
    external_provider: readStringField(activeTurn, "external_provider"),
    external_provider_session_id: readStringField(activeTurn, "external_provider_session_id"),
  }) ?? undefined
  const activeTurnExternalProvider = readNonBlankStringField(activeTurn, "external_provider") ?? undefined
  const activeTurnExternalProviderSessionId = readNonBlankStringField(activeTurn, "external_provider_session_id") ?? undefined
  const activeTurnExternalProviderTurnId = readNonBlankStringField(activeTurn, "external_provider_turn_id") ?? undefined
  const activeTurnStatus = normalizeAgentRuntimePromptProjectionStatus(readStringField(activeTurn, "status")) ?? undefined
  const activeTurnPhase = readStringField(activeTurn, "phase") ?? undefined
  const activeTurnStartedAtMs = readNumberField(activeTurn, "started_at_ms") ?? undefined
  return {
    ...(activeTurnPromptId ? { activeTurnPromptId } : {}),
    ...(activeTurnProviderRunId ? { activeTurnProviderRunId } : {}),
    ...(activeTurnPromptOrigin ? { activeTurnPromptOrigin } : {}),
    ...(activeTurnExternalProvider ? { activeTurnExternalProvider } : {}),
    ...(activeTurnExternalProviderSessionId ? { activeTurnExternalProviderSessionId } : {}),
    ...(activeTurnExternalProviderTurnId ? { activeTurnExternalProviderTurnId } : {}),
    ...(activeTurnStatus ? { activeTurnStatus } : {}),
    ...(activeTurnPhase ? { activeTurnPhase } : {}),
    ...(activeTurnStartedAtMs !== undefined ? { activeTurnStartedAtMs } : {}),
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
