import type { ManagedEnvironmentAutoStopPolicy, ManagedEnvironmentContextPlanInput } from "./ipc-managed-environment-requests.js"

export const disposableWorkerControlMinimumProtocolVersion = 351
export const managedEnvironmentKeepRunningMinimumProtocolVersion = 351

export type DisposableWorkerSelection = {
  readonly allocationId: string
  readonly homeKernelId: string
  readonly homeRelayRealmId: string
}

export type CreateDisposableWorkerInput = Omit<DisposableWorkerSelection, "allocationId"> & {
  readonly clientRequestId: string
  readonly region: string
  readonly computeClass: string
  readonly architecture: string
  readonly maximumLifetimeSeconds: number
  readonly autoStopPolicy: ManagedEnvironmentAutoStopPolicy
  readonly contextPlan?: ManagedEnvironmentContextPlanInput
}

export type DisposableWorkerAllocation = {
  readonly allocationId: string
  readonly homeKernelId: string | null
  readonly homeRelayRealmId: string | null
  readonly region: string
  readonly computeClass: string
  readonly architecture: string
  readonly providerId: string
  readonly providerProfileId: string
  readonly storageMonthlyMinorUnits: number | null
  readonly managedRepositoryRoot: string
  readonly runtimeReleaseDigest: string
  readonly desiredState: "running" | "released"
  readonly observedState: "requested" | "provisioning" | "enrolling" | "ready" | "leased" | "deleting" | "deleted" | "failed"
  readonly desiredRevision: number
  readonly observedRevision: number
  readonly runtimeMachineId: string | null
  readonly runtimeKernelId: string | null
  readonly operationId: string
  readonly operationStatus: "pending" | "running" | "succeeded" | "failed"
  readonly expiresAt: string
  readonly createdAt: string
  readonly autoStopPolicy: ManagedEnvironmentAutoStopPolicy
  readonly autoStopDeadlineAt: string | null
}

// Explicit projection prevents accidental forwarding of account/actor/credential fields.
function selection(input: DisposableWorkerSelection) {
  return { allocationId: input.allocationId, homeKernelId: input.homeKernelId, homeRelayRealmId: input.homeRelayRealmId }
}
export function createDisposableWorkerRequest(input: CreateDisposableWorkerInput) {
  return { CreateDisposableWorker: {
    clientRequestId: input.clientRequestId, homeKernelId: input.homeKernelId, homeRelayRealmId: input.homeRelayRealmId,
    region: input.region, computeClass: input.computeClass, architecture: input.architecture,
    maximumLifetimeSeconds: input.maximumLifetimeSeconds, autoStopPolicy: input.autoStopPolicy,
    ...(input.contextPlan === undefined ? {} : { contextPlan: input.contextPlan }),
  } }
}
export function getDisposableWorkerRequest(input: DisposableWorkerSelection) { return { GetDisposableWorker: selection(input) } }
export function releaseDisposableWorkerRequest(input: DisposableWorkerSelection) { return { ReleaseDisposableWorker: selection(input) } }
export function keepDisposableWorkerRunningRequest(input: DisposableWorkerSelection) { return { KeepDisposableWorkerRunning: selection(input) } }
export function prepareDisposableWorkerContextTransferRequest(input: DisposableWorkerSelection) { return { PrepareDisposableWorkerContextTransfer: selection(input) } }
export function keepManagedEnvironmentRunningRequest(environmentId: string) { return { KeepManagedEnvironmentRunning: { environmentId } } }
