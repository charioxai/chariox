import type { ManagedContextLaunchTarget, ManagedContextTransferStatus } from "@chariox/kernel-client/ipc-managed-context-requests"
import type { ManagedContextTransferTicket, ManagedEnvironmentCatalog, ManagedEnvironmentContextPlanInput, ManagedEnvironmentReimagePreflight, ManagedEnvironmentReimageResult, ManagedEnvironmentResult, ManagedEnvironmentSummary, ManagedEnvironmentPreReimageObservationAcknowledgement } from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { CloudControlProfile } from "./cloud-control-auth.js"
import type { LocalIpcClient } from "./ipc.js"
import type { createManagedEnvironmentRequest, requestManagedEnvironmentLifecycleRequest, requestManagedEnvironmentReimageRequest } from "./ipc-requests.js"
import { getManagedContextLaunchTargetRequest, getManagedContextTransferStatusRequest, observeManagedEnvironmentPreReimageRequest, preflightProviderAccountPortabilityRequest, providerAccountPortabilityPreflightMinimumProtocolVersion, startManagedContextTransferRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import { sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"
import { managedEnvironmentJson } from "./managed-environment-cloud-http.js"
import { prepareManagedEnvironmentReimageStop } from "./managed-environment-reimage-stop.js"

// MP-08 / MP-11: human control plane uses terminal authority; context export,
// import, local launch and old-generation observation stay kernel-owned.
export async function listManagedEnvironmentCatalog(profile: CloudControlProfile): Promise<ManagedEnvironmentCatalog> {
  const [options, environments] = await Promise.all([
    managedEnvironmentJson<Pick<ManagedEnvironmentCatalog, "computeClasses" | "contextSources">>(profile, "/managed-environments/options"),
    managedEnvironmentJson<Pick<ManagedEnvironmentCatalog, "environments">>(profile, "/managed-environments"),
  ])
  return {...options, ...environments}
}
export async function getManagedEnvironment(profile: CloudControlProfile, environmentId: string): Promise<ManagedEnvironmentSummary> {
  const {environment} = await managedEnvironmentJson<{environment: ManagedEnvironmentSummary}>(profile, `/managed-environments/${encodeURIComponent(environmentId)}`)
  if (environment.environmentId !== environmentId || environment.accountId !== profile.accountId) throw new Error("Cloud returned a different managed environment")
  return environment
}
export async function getManagedEnvironmentReimagePreflight(profile: CloudControlProfile, environmentId: string): Promise<ManagedEnvironmentReimagePreflight> {
  const preflight = await managedEnvironmentJson<ManagedEnvironmentReimagePreflight>(profile, `/managed-environments/${encodeURIComponent(environmentId)}/reimage/preflight`)
  if (preflight.environmentId !== environmentId) throw new Error("Cloud returned reimage preflight for a different managed environment")
  return preflight
}
export async function createManagedEnvironment(profile: CloudControlProfile, input: Parameters<typeof createManagedEnvironmentRequest>[0], client?: LocalIpcClient): Promise<ManagedEnvironmentResult> {
  await preflightProviderAccountPortability(client, input.contextPlan)
  const result = await managedEnvironmentJson<ManagedEnvironmentResult>(profile, "/managed-environments", input)
  validateManagedEnvironmentResult(result)
  if (result.environment.accountId !== profile.accountId) throw new Error("Cloud returned a different managed account")
  return result
}
export async function requestManagedEnvironmentLifecycle(profile: CloudControlProfile, input: Parameters<typeof requestManagedEnvironmentLifecycleRequest>[0]): Promise<ManagedEnvironmentResult> {
  const {environmentId, ...body} = input
  const result = await managedEnvironmentJson<ManagedEnvironmentResult>(profile, `/managed-environments/${encodeURIComponent(environmentId)}/lifecycle`, body)
  validateManagedEnvironmentResult(result, input.environmentId)
  if (result.environment.accountId !== profile.accountId) throw new Error("Cloud returned a different managed account")
  return result
}
export async function requestManagedEnvironmentReimage(profile: CloudControlProfile, input: Parameters<typeof requestManagedEnvironmentReimageRequest>[0], client?: LocalIpcClient): Promise<ManagedEnvironmentReimageResult> {
  await preflightProviderAccountPortability(client, input.contextPlan)
  await prepareManagedEnvironmentReimageStop(profile, input)
  const {environmentId, ...body} = input
  const result = await managedEnvironmentJson<ManagedEnvironmentReimageResult>(profile, `/managed-environments/${encodeURIComponent(environmentId)}/reimage`, body)
  validateManagedEnvironmentReimageResult(result, input)
  if (result.environment.accountId !== profile.accountId) throw new Error("Cloud returned a different managed account")
  return result
}
export async function prepareManagedEnvironmentContextTransfer(profile: CloudControlProfile, environmentId: string): Promise<ManagedContextTransferTicket> {
  const ticket = await managedEnvironmentJson<ManagedContextTransferTicket>(profile, `/managed-environments/${encodeURIComponent(environmentId)}/context-transfer`)
  if (ticket.environmentId !== environmentId) throw new Error("Cloud returned a context transfer ticket for a different managed environment")
  // StartManagedContextTransfer validates the ticket against the source kernel
  // identity before it reads, exports or sends any selected context.
  return ticket
}

export async function preflightProviderAccountPortability(client: LocalIpcClient | undefined, plan: ManagedEnvironmentContextPlanInput): Promise<void> {
  if (plan.providerAccounts.kind === "none") return
  if (!client) throw new Error("Selected provider accounts require a kernel credential portability preflight before provisioning")
  const response = await sendWithProtocolMinimum<Record<string, unknown>>(
    client.send.bind(client), preflightProviderAccountPortabilityRequest(plan.providerAccounts), {
      capability: "MP-08 / MP-11 provider account portability preflight",
      requestVariant: "PreflightProviderAccountPortability",
      minimumProtocolVersion: providerAccountPortabilityPreflightMinimumProtocolVersion,
    },
  )
  const acknowledgement = expectVariant<Record<string, never>>(response, "ProviderAccountPortabilityPreflightPassed")
  if (!acknowledgement || typeof acknowledgement !== "object" || Array.isArray(acknowledgement) || Object.keys(acknowledgement).length !== 0) {
    throw new Error("kernel returned an invalid provider portability acknowledgement")
  }
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

export async function startManagedContextTransfer(
  client: LocalIpcClient,
  ticket: ManagedContextTransferTicket,
): Promise<ManagedContextTransferStatus> {
  const response = await sendWithProtocolMinimum<Record<string, unknown>>(client.send.bind(client), startManagedContextTransferRequest(ticket, true), {
    capability: "MP-08 / MP-10 Project export review", requestVariant: "StartManagedContextTransfer", unknownField: "interactive", minimumProtocolVersion: 371,
  })
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
