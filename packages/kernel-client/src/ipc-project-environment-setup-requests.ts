import type { ProjectEnvironmentDefinition } from "./kernel-types-project-environment.js"

export function startProjectEnvironmentSetupRequest(input: {
  readonly operationId: string
  readonly projectId: string
  readonly sessionId: string
  readonly agentId: string
  readonly targetWorkerId: string
  readonly targetPlatform: string
  readonly definition?: ProjectEnvironmentDefinition | null
  readonly validationCommands?: readonly string[]
}) {
  return {
    StartProjectEnvironmentSetup: {
      operationId: input.operationId,
      projectId: input.projectId,
      sessionId: input.sessionId,
      agentId: input.agentId,
      targetWorkerId: input.targetWorkerId,
      targetPlatform: input.targetPlatform,
      ...(input.definition ? { definition: input.definition } : {}),
      ...(input.validationCommands && input.validationCommands.length > 0
        ? { validationCommands: [...input.validationCommands] }
        : {}),
    },
  } as const
}

export function getProjectEnvironmentSetupStatusRequest(operationId: string) {
  return { GetProjectEnvironmentSetupStatus: { operationId } } as const
}

export function cancelProjectEnvironmentSetupRequest(operationId: string, sessionId: string) {
  return {
    CancelProjectEnvironmentSetup: { operationId, sessionId },
  } as const
}

export function retryProjectEnvironmentSetupRequest(operationId: string, sessionId: string) {
  return {
    RetryProjectEnvironmentSetup: { operationId, sessionId },
  } as const
}
