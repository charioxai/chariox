import type {
  ProjectEnvironmentDefinition,
  ProjectEnvironmentSetupPhase,
  ProjectEnvironmentSetupStatus,
} from "@chariox/kernel-client/kernel-types"
import {
  cancelProjectEnvironmentSetupRequest,
  getProjectEnvironmentSetupStatusRequest,
  retryProjectEnvironmentSetupRequest,
  startProjectEnvironmentSetupRequest,
} from "@chariox/kernel-client/ipc-project-environment-setup-requests"

const PROJECT_ENVIRONMENT_SETUP_PHASES: readonly ProjectEnvironmentSetupPhase[] = [
  "requested",
  "preparing",
  "validating",
  "ready",
  "failed",
  "cancelled",
]

const MAX_DISPLAY_TEXT_LENGTH = 240
const MAX_DISPLAY_OPERATION_ID_LENGTH = 96

export type ProjectEnvironmentSetupRequestClient = {
  send<TResponse>(request: unknown): Promise<TResponse>
}

export type ProjectEnvironmentSetupStartInput = {
  readonly operationId: string
  readonly projectId: string
  readonly sessionId: string
  readonly agentId: string
  readonly targetWorkerId: string
  readonly targetPlatform: string
  readonly definition?: ProjectEnvironmentDefinition | null
  readonly validationCommands?: readonly string[]
}

export type ProjectEnvironmentSetupProjection = {
  status(): ProjectEnvironmentSetupStatus | null
  lastError(): string | null
  adopt(status: ProjectEnvironmentSetupStatus): ProjectEnvironmentSetupStatus
  start(input: ProjectEnvironmentSetupStartInput): Promise<ProjectEnvironmentSetupStatus>
  poll(): Promise<ProjectEnvironmentSetupStatus | null>
  cancel(): Promise<ProjectEnvironmentSetupStatus | null>
  retry(): Promise<ProjectEnvironmentSetupStatus | null>
  clear(): void
}

type ProjectEnvironmentSetupResponse = {
  ProjectEnvironmentSetupStarted?: { status?: unknown }
  ProjectEnvironmentSetupStatus?: { status?: unknown }
  ProjectEnvironmentSetupCancelled?: { status?: unknown }
  ProjectEnvironmentSetupRetried?: { status?: unknown }
}

export type ProjectEnvironmentSetupProjectionDeps = {
  client: ProjectEnvironmentSetupRequestClient
  onStatusChanged?: (status: ProjectEnvironmentSetupStatus | null) => void
  onError?: (operation: string, error: unknown) => void
}

export function createProjectEnvironmentSetupProjection(
  deps: ProjectEnvironmentSetupProjectionDeps,
): ProjectEnvironmentSetupProjection {
  let currentStatus: ProjectEnvironmentSetupStatus | null = null
  let currentError: string | null = null
  let revision = 0
  let pendingPoll: Promise<ProjectEnvironmentSetupStatus | null> | null = null

  const publish = (status: ProjectEnvironmentSetupStatus | null) => {
    currentStatus = status
    deps.onStatusChanged?.(status)
    return status
  }

  const publishServerStatus = (value: unknown, expectedOperationId?: string) => {
    const status = normalizeProjectEnvironmentSetupStatus(value)
    if (expectedOperationId && status.operation_id !== expectedOperationId) {
      throw new Error("project environment setup response changed operation identity")
    }
    currentError = null
    return publish(status)
  }

  const reportError = (operation: string, error: unknown) => {
    currentError = safeProjectEnvironmentSetupTransportError(error)
    deps.onError?.(operation, error)
  }

  const responseStatus = (response: unknown) => {
    if (isRecord(response)) {
      for (const key of [
        "ProjectEnvironmentSetupStarted",
        "ProjectEnvironmentSetupStatus",
        "ProjectEnvironmentSetupCancelled",
        "ProjectEnvironmentSetupRetried",
      ] as const) {
        const envelope = response[key]
        if (isRecord(envelope) && "status" in envelope) {
          return envelope.status
        }
      }
    }
    if (isRecord(response) && "operation_id" in response && "phase" in response) {
      return response
    }
    throw new Error("project environment setup response did not contain a status")
  }

  const start = async (input: ProjectEnvironmentSetupStartInput) => {
    const operationRevision = ++revision
    currentError = null
    try {
      const response = await deps.client.send<ProjectEnvironmentSetupResponse>(
        startProjectEnvironmentSetupRequest(input),
      )
      if (operationRevision !== revision) {
        return currentStatus ?? normalizeProjectEnvironmentSetupStatus(responseStatus(response))
      }
      return publishServerStatus(responseStatus(response), input.operationId)
    } catch (error) {
      reportError("start project environment setup", error)
      throw error
    }
  }

  const poll = () => {
    const operationId = currentStatus?.operation_id
    if (!operationId) {
      return Promise.resolve(null)
    }
    if (pendingPoll) {
      return pendingPoll
    }
    const operationRevision = revision
    const nextPoll = (async () => {
      try {
        const response = await deps.client.send<ProjectEnvironmentSetupResponse>(
          getProjectEnvironmentSetupStatusRequest(operationId),
        )
        if (operationRevision !== revision || currentStatus?.operation_id !== operationId) {
          return currentStatus
        }
        return publishServerStatus(responseStatus(response), operationId)
      } catch (error) {
        reportError("poll project environment setup", error)
        return currentStatus
      } finally {
        pendingPoll = null
      }
    })()
    pendingPoll = nextPoll
    return nextPoll
  }

  const runControl = async (
    operation: "cancel" | "retry",
  ): Promise<ProjectEnvironmentSetupStatus | null> => {
    const status = currentStatus
    if (!status) {
      return null
    }
    const operationRevision = ++revision
    currentError = null
    const request = operation === "cancel"
      ? cancelProjectEnvironmentSetupRequest(status.operation_id, status.session_id)
      : retryProjectEnvironmentSetupRequest(status.operation_id, status.session_id)
    try {
      const response = await deps.client.send<ProjectEnvironmentSetupResponse>(request)
      if (operationRevision !== revision) {
        return currentStatus
      }
      return publishServerStatus(responseStatus(response), status.operation_id)
    } catch (error) {
      reportError(`${operation} project environment setup`, error)
      throw error
    }
  }

  return {
    status: () => currentStatus,
    lastError: () => currentError,
    adopt(status: ProjectEnvironmentSetupStatus) {
      revision += 1
      currentError = null
      return publishServerStatus(status)
    },
    start,
    poll,
    cancel: () => runControl("cancel"),
    retry: () => runControl("retry"),
    clear() {
      revision += 1
      currentError = null
      publish(null)
    },
  }
}

export function normalizeProjectEnvironmentSetupStatus(value: unknown): ProjectEnvironmentSetupStatus {
  if (!isRecord(value)) {
    throw new Error("project environment setup status is not an object")
  }
  const operationId = requiredString(value.operation_id, "operation_id")
  const phase = value.phase
  if (!isProjectEnvironmentSetupPhase(phase)) {
    throw new Error("project environment setup status has an unknown phase")
  }
  return {
    ...value,
    operation_id: operationId,
    project_id: requiredString(value.project_id, "project_id"),
    session_id: requiredString(value.session_id, "session_id"),
    agent_id: requiredString(value.agent_id, "agent_id"),
    worker_id: requiredString(value.worker_id, "worker_id"),
    platform: requiredString(value.platform, "platform"),
    phase,
    attempt: boundedNonNegativeInteger(value.attempt),
    progress_percent: boundedProgressPercent(value.progress_percent),
    definition_digest: optionalString(value.definition_digest),
    validation: value.validation ?? null,
    message: safeOptionalText(value.message),
    failure_code: safeOptionalText(value.failure_code),
    failure_message: safeOptionalText(value.failure_message),
    retryable: value.retryable === true,
    created_at_ms: boundedNonNegativeInteger(value.created_at_ms),
    updated_at_ms: boundedNonNegativeInteger(value.updated_at_ms),
  } as ProjectEnvironmentSetupStatus
}

export function isProjectEnvironmentSetupPhase(value: unknown): value is ProjectEnvironmentSetupPhase {
  return typeof value === "string" && PROJECT_ENVIRONMENT_SETUP_PHASES.includes(value as ProjectEnvironmentSetupPhase)
}

export function boundedProgressPercent(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return 0
  }
  return Math.max(0, Math.min(100, Math.round(value)))
}

export function safeProjectEnvironmentSetupFailureDetails(
  status: Pick<ProjectEnvironmentSetupStatus, "phase" | "failure_code" | "failure_message">,
): string | null {
  if (status.phase !== "failed") {
    return null
  }
  const code = safeDisplayText(status.failure_code)
  const message = safeDisplayText(status.failure_message)
  if (code && message) {
    return truncateDisplayText(`${code}: ${message}`)
  }
  return truncateDisplayText(code || message || "project environment setup failed")
}

export function safeProjectEnvironmentSetupTransportError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return truncateDisplayText(message) || "project environment setup is temporarily unavailable"
}

let activeProjection: ProjectEnvironmentSetupProjection | null = null

export function setActiveProjectEnvironmentSetupProjection(
  projection: ProjectEnvironmentSetupProjection | null,
) {
  activeProjection = projection
  return () => {
    if (activeProjection === projection) {
      activeProjection = null
    }
  }
}

export function activeProjectEnvironmentSetupProjection() {
  return activeProjection
}

export function activeProjectEnvironmentSetupStatus() {
  return activeProjection?.status() ?? null
}

export async function pollActiveProjectEnvironmentSetup(): Promise<void> {
  await activeProjection?.poll()
}

function isRecord(value: unknown): value is Record<string, any> {
  return typeof value === "object" && value !== null
}

function requiredString(value: unknown, name: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`project environment setup status is missing ${name}`)
  }
  return value
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null
}

function boundedNonNegativeInteger(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return 0
  }
  return Math.max(0, Math.floor(value))
}

function safeOptionalText(value: unknown): string | null {
  const text = safeDisplayText(value)
  return text || null
}

function safeDisplayText(value: unknown): string {
  if (typeof value !== "string") {
    return ""
  }
  return truncateDisplayText(value.replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim())
}

function truncateDisplayText(value: string): string {
  return value.length > MAX_DISPLAY_TEXT_LENGTH
    ? `${value.slice(0, MAX_DISPLAY_TEXT_LENGTH - 1)}…`
    : value
}

export function safeProjectEnvironmentSetupOperationId(value: string): string {
  const compact = value.replace(/[^a-zA-Z0-9._:-]/g, "-")
  return compact.length > MAX_DISPLAY_OPERATION_ID_LENGTH
    ? `${compact.slice(0, MAX_DISPLAY_OPERATION_ID_LENGTH - 1)}…`
    : compact
}
