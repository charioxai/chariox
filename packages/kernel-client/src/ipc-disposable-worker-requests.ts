import type { ManagedEnvironmentAutoStopPolicy, ManagedEnvironmentContextPlanInput } from "./ipc-managed-environment-requests.js"

export const disposableWorkerControlMinimumProtocolVersion = 366
export const managedEnvironmentKeepRunningMinimumProtocolVersion = 366

const disposableControlRequests = new Set([
  "CreateDisposableWorker", "GetDisposableWorker", "ReleaseDisposableWorker",
  "KeepDisposableWorkerRunning", "PrepareDisposableWorkerContextTransfer",
])

export function isGuardedKernelControl(request: unknown): boolean {
  return Object.keys(record(request) ?? {}).some(name =>
    disposableControlRequests.has(name) || name === "KeepManagedEnvironmentRunning")
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : undefined
}

// Query the connected kernel, not relay discovery metadata. Parallel development
// branches can reuse a numeric protocol version without implementing this contract.
export async function requireKernelControlCapability(
  send: (request: unknown) => Promise<unknown>,
  request: unknown,
  expected?: { daemonId: string; machineId?: string },
): Promise<void> {
  const envelope = record(request)
  if (!envelope) return
  const names = Object.keys(envelope)
  const name = names.find(value => disposableControlRequests.has(value) || value === "KeepManagedEnvironmentRunning")
  if (!name) return
  if (names.length !== 1) throw new Error("Invalid kernel control request")
  const payload = record(envelope[name])
  const home = payload?.homeKernelId
  if (disposableControlRequests.has(name) && (typeof home !== "string" || !home.trim())) {
    throw new Error("Kernel control request requires a home kernel identity")
  }
  const response = record(await send({ RelayStatus: null }))
  const status = record(record(response?.RelayStatus)?.status)
  if (!status || typeof status.daemon_id !== "string" || !status.daemon_id
    || typeof status.machine_id !== "string" || !status.machine_id) {
    throw new Error("Kernel control capability status is unavailable")
  }
  if ((home !== undefined && status.daemon_id !== home)
    || (expected && status.daemon_id !== expected.daemonId)) {
    throw new Error("Kernel control capability status belongs to another kernel")
  }
  if (expected?.machineId !== undefined && status.machine_id !== expected.machineId) {
    throw new Error("Kernel control capability status belongs to another machine")
  }
  const capability = name === "KeepManagedEnvironmentRunning"
    ? "managed_environment_keep_running_v1" : "disposable_worker_control_v1"
  if (!Array.isArray(status.capabilities)
    || !status.capabilities.every(value => typeof value === "string")
    || !status.capabilities.includes(capability)) {
    throw new Error(`Connected kernel does not support ${capability}; update the kernel before retrying`)
  }
}

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
