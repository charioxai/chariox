import type {
  ManagedContextLaunchTarget,
  ManagedContextTransferStatus,
} from "@chariox/kernel-client/ipc-managed-context-requests"
import type {
  ManagedContextTransferTicket,
  ManagedEnvironmentCatalog,
  ManagedEnvironmentContextPlanInput,
  ManagedEnvironmentLifecycleAction,
  ManagedEnvironmentResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { LocalIpcClient } from "./ipc.js"
import {
  createManagedEnvironmentRequest,
  getManagedContextLaunchTargetRequest,
  getManagedContextTransferStatusRequest,
  getManagedEnvironmentRequest,
  listManagedEnvironmentCatalogRequest,
  prepareManagedEnvironmentContextTransferRequest,
  requestManagedEnvironmentLifecycleRequest,
  startManagedContextTransferRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function listManagedEnvironmentCatalog(
  client: LocalIpcClient,
): Promise<ManagedEnvironmentCatalog> {
  const response = await client.send<Record<string, unknown>>(listManagedEnvironmentCatalogRequest())
  return expectVariant<{ catalog: ManagedEnvironmentCatalog }>(
    response,
    "ManagedEnvironmentCatalog",
  ).catalog
}

export async function getManagedEnvironment(
  client: LocalIpcClient,
  environmentId: string,
): Promise<ManagedEnvironmentSummary> {
  const response = await client.send<Record<string, unknown>>(getManagedEnvironmentRequest(environmentId))
  const environment = expectVariant<{ environment: ManagedEnvironmentSummary }>(
    response,
    "ManagedEnvironment",
  ).environment
  if (environment.environmentId !== environmentId) {
    throw new Error("kernel returned a different managed environment")
  }
  return environment
}

export async function createManagedEnvironment(
  client: LocalIpcClient,
  input: {
    clientRequestId: string
    name: string
    region: string
    computeClass: string
    autoStopPolicy: { minimumRuntimeSeconds: number; idleDelaySeconds: number | null }
    contextPlan: ManagedEnvironmentContextPlanInput
  },
): Promise<ManagedEnvironmentResult> {
  const response = await client.send<Record<string, unknown>>(createManagedEnvironmentRequest(input))
  const result = expectVariant<{ result: ManagedEnvironmentResult }>(
    response,
    "ManagedEnvironmentCreated",
  ).result
  validateManagedEnvironmentResult(result)
  return result
}

export async function requestManagedEnvironmentLifecycle(
  client: LocalIpcClient,
  input: {
    environmentId: string
    action: ManagedEnvironmentLifecycleAction
    idempotencyKey: string
  },
): Promise<ManagedEnvironmentResult> {
  const response = await client.send<Record<string, unknown>>(
    requestManagedEnvironmentLifecycleRequest(input),
  )
  const result = expectVariant<{ result: ManagedEnvironmentResult }>(
    response,
    "ManagedEnvironmentLifecycleRequested",
  ).result
  validateManagedEnvironmentResult(result, input.environmentId)
  return result
}

export async function prepareManagedEnvironmentContextTransfer(
  client: LocalIpcClient,
  environmentId: string,
): Promise<ManagedContextTransferTicket> {
  const response = await client.send<Record<string, unknown>>(
    prepareManagedEnvironmentContextTransferRequest(environmentId),
  )
  const ticket = expectVariant<{ ticket: ManagedContextTransferTicket }>(
    response,
    "ManagedEnvironmentContextTransferPrepared",
  ).ticket
  if (ticket.environmentId !== environmentId) {
    throw new Error("kernel returned a context transfer ticket for a different managed environment")
  }
  return ticket
}

export async function startManagedContextTransfer(
  client: LocalIpcClient,
  ticket: ManagedContextTransferTicket,
): Promise<ManagedContextTransferStatus> {
  const response = await client.send<Record<string, unknown>>(startManagedContextTransferRequest(ticket))
  return expectVariant<{ status: ManagedContextTransferStatus }>(
    response,
    "ManagedContextTransferStarted",
  ).status
}

export async function getManagedContextTransferStatus(
  client: LocalIpcClient,
  contextId: string,
): Promise<ManagedContextTransferStatus> {
  const response = await client.send<Record<string, unknown>>(
    getManagedContextTransferStatusRequest(contextId),
  )
  return expectVariant<{ status: ManagedContextTransferStatus }>(
    response,
    "ManagedContextTransferStatus",
  ).status
}

export async function getManagedContextLaunchTarget(
  client: LocalIpcClient,
  contextId: string,
  planDigest: string,
): Promise<ManagedContextLaunchTarget> {
  const response = await client.send<Record<string, unknown>>(
    getManagedContextLaunchTargetRequest(contextId, planDigest),
  )
  return expectVariant<{ target: ManagedContextLaunchTarget }>(
    response,
    "ManagedContextLaunchTarget",
  ).target
}

function validateManagedEnvironmentResult(
  result: ManagedEnvironmentResult,
  expectedEnvironmentId?: string,
): void {
  if (result.operation.environmentId !== result.environment.environmentId
    || (expectedEnvironmentId !== undefined && result.environment.environmentId !== expectedEnvironmentId)) {
    throw new Error("kernel returned a mismatched managed environment result")
  }
}
