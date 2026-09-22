import type {
  ManagedContextLaunchTarget,
  ManagedContextTransferStatus,
} from "@chariox/kernel-client/ipc-managed-context-requests"
import {
  managedEnvironmentReimagePreflightMinimumProtocolVersion,
  type ManagedContextTransferTicket,
  type ManagedEnvironmentCatalog,
  type ManagedEnvironmentContextPlanInput,
  type ManagedEnvironmentLifecycleAction,
  type ManagedEnvironmentPreReimageObservationAcknowledgement,
  type ManagedEnvironmentReimagePreflight,
  type ManagedEnvironmentReimageResult,
  type ManagedEnvironmentResult,
  type ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { LocalIpcClient } from "./ipc.js"
import {
  createManagedEnvironmentRequest,
  getManagedContextLaunchTargetRequest,
  getManagedContextTransferStatusRequest,
  getManagedEnvironmentRequest,
  getManagedEnvironmentReimagePreflightRequest,
  listManagedEnvironmentCatalogRequest,
  observeManagedEnvironmentPreReimageRequest,
  prepareManagedEnvironmentContextTransferRequest,
  requestManagedEnvironmentLifecycleRequest,
  requestManagedEnvironmentReimageRequest,
  startManagedContextTransferRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import { sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"

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

export async function getManagedEnvironmentReimagePreflight(
  client: LocalIpcClient,
  environmentId: string,
): Promise<ManagedEnvironmentReimagePreflight> {
  const response = await sendWithProtocolMinimum<Record<string, unknown>>(
    client.send.bind(client),
    getManagedEnvironmentReimagePreflightRequest(environmentId),
    {
      capability: "Managed environment reimage preflight",
      requestVariant: "GetManagedEnvironmentReimagePreflight",
      minimumProtocolVersion: managedEnvironmentReimagePreflightMinimumProtocolVersion,
    },
  )
  const preflight = expectVariant<{ preflight: ManagedEnvironmentReimagePreflight }>(
    response,
    "ManagedEnvironmentReimagePreflight",
  ).preflight
  if (preflight.environmentId !== environmentId) {
    throw new Error("kernel returned reimage preflight for a different managed environment")
  }
  return preflight
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

export async function requestManagedEnvironmentReimage(
  client: LocalIpcClient,
  input: {
    environmentId: string
    expectedGeneration: number
    expectedProviderServerId: string
    expectedProviderImageId: string
    expectedProviderProfileId: string
    expectedProviderProfileDigest: string
    expectedRuntimeReleaseDigest: string
    expectedRuntimeSourceCommit: string
    expectedRuntimeSourceTree: string
    contextPlan: ManagedEnvironmentContextPlanInput
    idempotencyKey: string
  },
): Promise<ManagedEnvironmentReimageResult> {
  const response = await client.send<Record<string, unknown>>(
    requestManagedEnvironmentReimageRequest(input),
  )
  const result = expectVariant<{ result: ManagedEnvironmentReimageResult }>(
    response,
    "ManagedEnvironmentReimageRequested",
  ).result
  validateManagedEnvironmentReimageResult(result, input)
  return result
}

export async function observeManagedEnvironmentPreReimage(
  client: LocalIpcClient,
  input: {
    environmentId: string
    expectedGeneration: number
  },
): Promise<ManagedEnvironmentPreReimageObservationAcknowledgement> {
  const response = await client.send<Record<string, unknown>>(
    observeManagedEnvironmentPreReimageRequest(input),
  )
  const acknowledgement = expectVariant<{
    acknowledgement: ManagedEnvironmentPreReimageObservationAcknowledgement
  }>(response, "ManagedEnvironmentPreReimageObserved").acknowledgement
  if (acknowledgement.environmentId !== input.environmentId
    || acknowledgement.generation !== input.expectedGeneration
    || acknowledgement.observedAt.trim() === "") {
    throw new Error("kernel returned a mismatched managed pre-reimage observation acknowledgement")
  }
  return acknowledgement
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

function validateManagedEnvironmentReimageResult(
  result: ManagedEnvironmentReimageResult,
  input: {
    environmentId: string
    expectedGeneration: number
    expectedProviderServerId: string
    expectedProviderImageId: string
    expectedProviderProfileId: string
    expectedProviderProfileDigest: string
    expectedRuntimeReleaseDigest: string
    expectedRuntimeSourceCommit: string
    expectedRuntimeSourceTree: string
    idempotencyKey: string
  },
): void {
  const receipt = result.receipt
  if (result.environment.environmentId !== input.environmentId
    || result.operation.environmentId !== input.environmentId
    || result.operation.kind !== "reimage"
    || result.operation.idempotencyKey !== input.idempotencyKey
    || receipt.environmentId !== input.environmentId
    || receipt.operationId !== result.operation.operationId
    || receipt.previousGeneration !== input.expectedGeneration
    || receipt.generation !== input.expectedGeneration + 1
    || receipt.receiptId.trim() === ""
    || receipt.providerServerId !== input.expectedProviderServerId
    || receipt.providerImageId !== input.expectedProviderImageId
    || receipt.providerProfileId !== input.expectedProviderProfileId
    || receipt.providerProfileDigest !== input.expectedProviderProfileDigest
    || receipt.runtimeReleaseDigest !== input.expectedRuntimeReleaseDigest
    || !sourceEvidenceMatchesRequest(receipt.sourceEvidence, input)) {
    throw new Error(
      "kernel returned managed reimage evidence that does not match the requested generation or exact provider identity",
    )
  }
}

function sourceEvidenceMatchesRequest(
  sourceEvidence: unknown,
  input: {
    expectedProviderImageId: string
    expectedProviderProfileId: string
    expectedProviderProfileDigest: string
    expectedRuntimeReleaseDigest: string
    expectedRuntimeSourceCommit: string
    expectedRuntimeSourceTree: string
  },
): boolean {
  if (sourceEvidence === null || typeof sourceEvidence !== "object" || Array.isArray(sourceEvidence)) {
    return false
  }
  const evidence = sourceEvidence as Record<string, unknown>
  return evidence.providerImageId === input.expectedProviderImageId
    && evidence.providerProfileId === input.expectedProviderProfileId
    && evidence.providerProfileDigest === input.expectedProviderProfileDigest
    && evidence.runtimeReleaseDigest === input.expectedRuntimeReleaseDigest
    && evidence.runtimeSourceCommit === input.expectedRuntimeSourceCommit
    && evidence.runtimeSourceTree === input.expectedRuntimeSourceTree
}
