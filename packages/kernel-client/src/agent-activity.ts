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
  readonly activePromptCount: number
  readonly queuedPromptCount: number
  readonly error: boolean
}

export function agentRuntimeActivityIsBusy(
  activity: AgentRuntimeActivityBusyInput | null | undefined,
): boolean {
  return projectAgentRuntimeActivity(activity).busy
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
  return {
    status,
    promptStatus,
    busy: rawBusy || activePromptCount > 0 || queuedPromptCount > 0,
    activeTurn,
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

function readNonNegativeIntegerField(value: unknown, field: string): number | null {
  const record = readRecord(value)
  const candidate = record?.[field]
  return typeof candidate === "number" && Number.isInteger(candidate) && candidate >= 0
    ? candidate
    : null
}
