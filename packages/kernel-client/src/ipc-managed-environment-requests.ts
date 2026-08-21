export type ManagedEnvironmentDesiredState = "running" | "stopped" | "deleted"

export type ManagedEnvironmentObservedState =
  | "requested"
  | "provisioning"
  | "bootstrapping"
  | "awaiting_context"
  | "ready"
  | "starting"
  | "stopping"
  | "stopped"
  | "deleting"
  | "deleted"
  | "failed"

export type ManagedEnvironmentLifecycleAction = "start" | "stop" | "restart" | "delete"

export type ManagedEnvironmentAutoStopPolicy = {
  readonly minimumRuntimeSeconds: number
  readonly idleDelaySeconds: number | null
}

export type ManagedEnvironmentRepositorySelection = {
  readonly role: "primary" | "supporting"
  readonly workspaceId: string
  readonly worktreeId: string | null
}

export type ManagedEnvironmentDevelopmentSetup =
  | { readonly kind: "empty" }
  | {
      readonly kind: "source_project"
      readonly projectId: string
      readonly repositories: readonly ManagedEnvironmentRepositorySelection[]
    }

export type ManagedEnvironmentProviderAccounts =
  | { readonly kind: "none" }
  | {
      readonly kind: "selected"
      readonly accounts: readonly {
        readonly provider: string
        readonly accountProfile: string
      }[]
    }

export type ManagedEnvironmentGitCredentials =
  | { readonly kind: "none" }
  | { readonly kind: "selected"; readonly credentialIds: readonly string[] }

export type ManagedEnvironmentContextPlanInput = {
  readonly sourceTargetId: string | null
  readonly kernelContext: "empty" | "source_kernel"
  readonly developmentSetup: ManagedEnvironmentDevelopmentSetup
  readonly providerAccounts: ManagedEnvironmentProviderAccounts
  readonly gitCredentials: ManagedEnvironmentGitCredentials
}

export type ManagedEnvironmentContextPlan = Omit<ManagedEnvironmentContextPlanInput, "sourceTargetId"> & {
  readonly schemaVersion: 1
  readonly contextId: string
  readonly planDigest: string
  readonly source: {
    readonly sourceTargetId: string
    readonly relayRealmId: string
    readonly machineId: string
    readonly kernelId: string
    readonly keyThumbprint: string
  } | null
}

export type ManagedEnvironmentComputeClassOption = {
  readonly computeClass: string
  readonly regions: readonly string[]
}

export type ManagedEnvironmentContextSourceOption = {
  readonly sourceTargetId: string
  readonly machineId: string
  readonly kernelId: string
  readonly label: string
}

export type ManagedEnvironmentSummary = {
  readonly environmentId: string
  readonly accountId: string
  readonly createdByUserId: string
  readonly name: string
  readonly region: string
  readonly computeClass: string
  readonly desiredState: ManagedEnvironmentDesiredState
  readonly observedState: ManagedEnvironmentObservedState
  readonly desiredRevision: number
  readonly observedRevision: number
  readonly runtimeMachineId: string | null
  readonly runtimeKernelId: string | null
  readonly runtimeReleaseDigest: string | null
  readonly contextPlan: ManagedEnvironmentContextPlan
  readonly contextManifestDigest: string | null
  readonly autoStopPolicy: ManagedEnvironmentAutoStopPolicy
  readonly lastErrorCode: string | null
  readonly lastErrorMessage: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export type ManagedEnvironmentOperationSummary = {
  readonly operationId: string
  readonly environmentId: string
  readonly requestedByUserId: string
  readonly kind: "create" | ManagedEnvironmentLifecycleAction
  readonly idempotencyKey: string
  readonly requestDigest: string
  readonly desiredRevision: number
  readonly status: "pending" | "running" | "succeeded" | "failed"
  readonly attempt: number
  readonly retryable: boolean
  readonly failureCode: string | null
  readonly failureMessage: string | null
  readonly completedAt: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export type ManagedEnvironmentResult = {
  readonly environment: ManagedEnvironmentSummary
  readonly operation: ManagedEnvironmentOperationSummary
}

export type ManagedEnvironmentCatalog = {
  readonly computeClasses: readonly ManagedEnvironmentComputeClassOption[]
  readonly contextSources: readonly ManagedEnvironmentContextSourceOption[]
  readonly environments: readonly ManagedEnvironmentSummary[]
}

export function listManagedEnvironmentCatalogRequest() {
  return { ListManagedEnvironmentCatalog: null } as const
}

export function getManagedEnvironmentRequest(environmentId: string) {
  return { GetManagedEnvironment: { environmentId } } as const
}

export function createManagedEnvironmentRequest(input: {
  readonly clientRequestId: string
  readonly name: string
  readonly region: string
  readonly computeClass: string
  readonly autoStopPolicy: ManagedEnvironmentAutoStopPolicy
  readonly contextPlan: ManagedEnvironmentContextPlanInput
}) {
  return { CreateManagedEnvironment: input } as const
}

export function requestManagedEnvironmentLifecycleRequest(input: {
  readonly environmentId: string
  readonly action: ManagedEnvironmentLifecycleAction
  readonly idempotencyKey: string
}) {
  return { RequestManagedEnvironmentLifecycle: input } as const
}
