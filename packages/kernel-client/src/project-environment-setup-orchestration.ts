import type {
  ProjectEnvironmentDefinition,
  ProjectEnvironmentSetupStatus,
} from "./kernel-types-project-environment.js"

const MAX_OPERATION_ID_CHARS = 128

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

export type ProjectEnvironmentSetupOperations = {
  start(input: ProjectEnvironmentSetupStartInput): Promise<ProjectEnvironmentSetupStatus>
  get(operationId: string): Promise<ProjectEnvironmentSetupStatus>
}

export type ProjectEnvironmentSetupReadinessOptions = {
  assertActive?(): void
  delay?(ms: number): Promise<void>
  nowMs?(): number
  pollIntervalMs?: number
  timeoutMs?: number
  onStatus?(status: ProjectEnvironmentSetupStatus): void
  onTransportError?(operation: "start" | "get", error: unknown): void
  isRetryableTransportError?(error: unknown): boolean
}

export function managedLaunchProjectEnvironmentSetupOperationId(sessionId: string): string {
  const normalizedSessionId = sessionId.trim()
  const operationId = `managed-launch-project-setup:${normalizedSessionId}`
  if (!normalizedSessionId || operationId.length > MAX_OPERATION_ID_CHARS) {
    throw new Error("managed launch Project setup operation identity is invalid")
  }
  return operationId
}

export async function ensureProjectEnvironmentSetupReady(
  operations: ProjectEnvironmentSetupOperations,
  input: ProjectEnvironmentSetupStartInput,
  options: ProjectEnvironmentSetupReadinessOptions = {},
): Promise<ProjectEnvironmentSetupStatus> {
  const assertActive = options.assertActive ?? (() => {})
  const delay = options.delay ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)))
  const nowMs = options.nowMs ?? Date.now
  const deadline = nowMs() + (options.timeoutMs ?? 15 * 60 * 1_000)
  const pollIntervalMs = options.pollIntervalMs ?? 1_500
  let status: ProjectEnvironmentSetupStatus | null = null

  assertActive()
  try {
    status = await operations.start(input)
  } catch (error) {
    assertActive()
    options.onTransportError?.("start", error)
    if (!options.isRetryableTransportError?.(error)) {
      throw error
    }
  }

  while (true) {
    assertActive()
    if (status) {
      assertProjectEnvironmentSetupBinding(status, input)
      options.onStatus?.(status)
      if (status.phase === "ready") {
        return status
      }
      if (status.phase === "cancelled") {
        throw new Error("Project environment setup was cancelled")
      }
      if (status.phase === "failed" && !status.retryable) {
        throw new Error(projectEnvironmentSetupFailureMessage(status))
      }
    }
    if (nowMs() >= deadline) {
      throw new Error(status?.phase === "failed"
        ? projectEnvironmentSetupFailureMessage(status)
        : "Timed out waiting for Project environment setup")
    }

    await delay(pollIntervalMs)
    assertActive()
    try {
      status = await operations.get(input.operationId)
    } catch (error) {
      assertActive()
      options.onTransportError?.("get", error)
      if (!options.isRetryableTransportError?.(error)) {
        throw error
      }
    }
  }
}

function assertProjectEnvironmentSetupBinding(
  status: ProjectEnvironmentSetupStatus,
  input: ProjectEnvironmentSetupStartInput,
) {
  if (status.operation_id !== input.operationId
    || status.project_id !== input.projectId
    || status.session_id !== input.sessionId
    || status.agent_id !== input.agentId
    || status.worker_id !== input.targetWorkerId
    || status.platform !== input.targetPlatform) {
    throw new Error("Project environment setup status changed the managed launch binding")
  }
}

function projectEnvironmentSetupFailureMessage(status: ProjectEnvironmentSetupStatus): string {
  return status.failure_message
    || status.failure_code
    || "Project environment setup failed"
}
