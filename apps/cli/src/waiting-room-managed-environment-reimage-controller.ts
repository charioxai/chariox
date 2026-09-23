import type {
  ManagedEnvironmentContextPlanInput,
  ManagedEnvironmentPreReimageObservationAcknowledgement,
  ManagedEnvironmentReimagePreflight,
  ManagedEnvironmentReimageResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"

export type ManagedEnvironmentReimageAttempt = {
  assertActive(): void
  progress(message: string): void
}

export type ManagedEnvironmentReimageConfirmation = {
  readonly environmentId: string
  readonly providerServerId: string
  readonly generation: number
  readonly runtimeMachineId: string
  readonly runtimeKernelId: string
  readonly providerImageId: string
  readonly providerProfileId: string
  readonly providerProfileDigest: string
  readonly runtimeReleaseDigest: string
  readonly runtimeSourceCommit: string
  readonly runtimeSourceTree: string
}

export type ManagedEnvironmentReimagePreparation =
  | { readonly status: "confirmation_required"; readonly confirmation: ManagedEnvironmentReimageConfirmation }
  | { readonly status: "resume_required"; readonly confirmation: ManagedEnvironmentReimageConfirmation }

export type ManagedEnvironmentReimageCancellation =
  | { readonly status: "cancelled" }
  | { readonly status: "nothing_pending" }
  | { readonly status: "irreversible"; readonly confirmation: ManagedEnvironmentReimageConfirmation }

type ImmutableReimageRequest = {
  readonly environmentId: string
  readonly expectedGeneration: number
  readonly expectedProviderServerId: string
  readonly expectedProviderImageId: string
  readonly expectedProviderProfileId: string
  readonly expectedProviderProfileDigest: string
  readonly expectedRuntimeReleaseDigest: string
  readonly expectedRuntimeSourceCommit: string
  readonly expectedRuntimeSourceTree: string
  readonly contextPlan: ManagedEnvironmentContextPlanInput
  readonly idempotencyKey: string
}

type PendingReimage = {
  readonly preflight: ManagedEnvironmentReimagePreflight
  readonly confirmation: ManagedEnvironmentReimageConfirmation
  readonly request: ImmutableReimageRequest
  phase: "awaiting_confirmation" | "irreversible"
}

export type WaitingRoomManagedEnvironmentReimageControllerDeps = {
  getPreflight(environmentId: string): Promise<ManagedEnvironmentReimagePreflight>
  observePreviousKernel(input: {
    environmentId: string
    expectedGeneration: number
    machineId: string
    kernelId: string
  }): Promise<ManagedEnvironmentPreReimageObservationAcknowledgement>
  requestReimage(input: ImmutableReimageRequest): Promise<ManagedEnvironmentReimageResult>
  getEnvironment(environmentId: string): Promise<ManagedEnvironmentSummary>
  launchReplacement(
    environment: ManagedEnvironmentSummary,
    attempt: ManagedEnvironmentReimageAttempt,
  ): Promise<void>
  createIdempotencyKey(): string
  delay(ms: number): Promise<void>
  nowMs(): number
  timeoutMs?: number
}

export class WaitingRoomManagedEnvironmentReimageController {
  private readonly pending = new Map<string, PendingReimage>()
  private readonly confirmations = new Map<string, Promise<ManagedEnvironmentSummary>>()

  constructor(private readonly deps: WaitingRoomManagedEnvironmentReimageControllerDeps) {}

  async prepare(
    environmentId: string,
    contextPlan: ManagedEnvironmentContextPlanInput,
    attempt: ManagedEnvironmentReimageAttempt,
  ): Promise<ManagedEnvironmentReimagePreparation> {
    const authorizedContextPlan = snapshotContextPlan(contextPlan)
    const existing = this.pending.get(environmentId)
    if (existing) {
      if (!contextPlanMatchesInput(existing.request.contextPlan, authorizedContextPlan)) {
        throw new Error("The selected reimage context changed; cancel the pending reimage before preparing another one.")
      }
      return {
        status: existing.phase === "irreversible" ? "resume_required" : "confirmation_required",
        confirmation: existing.confirmation,
      }
    }

    attempt.assertActive()
    attempt.progress("Reading the server-authoritative managed reimage preflight.")
    const preflight = snapshotPreflight(
      validatePreflight(await this.deps.getPreflight(environmentId), environmentId),
    )
    attempt.assertActive()
    attempt.progress("Observing the old managed kernel before reimage.")
    const acknowledgement = await this.deps.observePreviousKernel({
      environmentId,
      expectedGeneration: preflight.retained.generation,
      machineId: preflight.retained.runtimeMachineId,
      kernelId: preflight.retained.runtimeKernelId,
    })
    attempt.assertActive()
    validateObservation(acknowledgement, preflight)

    const confirmation = confirmationFromPreflight(preflight)
    this.pending.set(environmentId, {
      phase: "awaiting_confirmation",
      preflight,
      confirmation,
      request: requestFromPreflight(
        preflight,
        authorizedContextPlan,
        this.deps.createIdempotencyKey(),
      ),
    })
    return { status: "confirmation_required", confirmation }
  }

  async confirm(
    environmentId: string,
    attempt: ManagedEnvironmentReimageAttempt,
  ): Promise<ManagedEnvironmentSummary> {
    const pending = this.pending.get(environmentId)
    if (!pending) {
      throw new Error("Prepare this managed reimage before confirming its destructive mutation.")
    }
    if (pending.phase === "awaiting_confirmation") {
      attempt.assertActive()
      pending.phase = "irreversible"
    }
    const active = this.confirmations.get(environmentId)
    if (active) return await active
    const confirmation = this.resumeIrreversible(pending, attempt)
    this.confirmations.set(environmentId, confirmation)
    try {
      return await confirmation
    } finally {
      if (this.confirmations.get(environmentId) === confirmation) {
        this.confirmations.delete(environmentId)
      }
    }
  }

  cancel(environmentId: string): ManagedEnvironmentReimageCancellation {
    const pending = this.pending.get(environmentId)
    if (!pending) return { status: "nothing_pending" }
    if (pending.phase === "irreversible") {
      return { status: "irreversible", confirmation: pending.confirmation }
    }
    this.pending.delete(environmentId)
    return { status: "cancelled" }
  }

  private async resumeIrreversible(
    pending: PendingReimage,
    attempt: ManagedEnvironmentReimageAttempt,
  ): Promise<ManagedEnvironmentSummary> {
    const deadline = this.deps.nowMs() + (this.deps.timeoutMs ?? 30 * 60 * 1_000)
    while (true) {
      attempt.progress("Requesting the destructive managed-machine reimage.")
      const result = validateReimageResult(
        await this.deps.requestReimage(snapshotReimageRequest(pending.request)),
        pending,
      )
      if (result.operation.status === "failed" || result.receipt.status === "failed_closed") {
        throw new Error(
          result.receipt.failureMessage
          || result.operation.failureMessage
          || result.receipt.failureCode
          || result.operation.failureCode
          || "Managed reimage failed closed.",
        )
      }

      if (result.receipt.freshEquivalent && result.receipt.status === "fresh_equivalent") {
        const current = await this.deps.getEnvironment(pending.request.environmentId)
        const replacement = validateReadyReplacement(current, result, pending)
        attempt.progress("Connecting to the fresh managed kernel and preparing its Project.")
        await this.deps.launchReplacement(replacement, attempt)
        this.pending.delete(pending.request.environmentId)
        return replacement
      }

      if (this.deps.nowMs() >= deadline) {
        throw new Error(
          "Timed out waiting for the accepted managed reimage; rerun confirm to resume the same operation.",
        )
      }
      attempt.progress(`Waiting for managed reimage: ${result.receipt.status}.`)
      await this.deps.delay(1_500)
    }
  }
}

function snapshotContextPlan(
  contextPlan: ManagedEnvironmentContextPlanInput,
): ManagedEnvironmentContextPlanInput {
  return structuredClone(contextPlan)
}

function snapshotPreflight(
  preflight: ManagedEnvironmentReimagePreflight,
): ManagedEnvironmentReimagePreflight {
  return structuredClone(preflight)
}

function snapshotReimageRequest(request: ImmutableReimageRequest): ImmutableReimageRequest {
  return structuredClone(request)
}

function validatePreflight(
  preflight: ManagedEnvironmentReimagePreflight,
  environmentId: string,
): ManagedEnvironmentReimagePreflight {
  const retained = preflight.retained
  const desired = preflight.desiredRelease
  if (preflight.environmentId !== environmentId
    || !Number.isSafeInteger(retained.generation)
    || retained.generation < 0
    || retained.desiredRevision !== retained.observedRevision
    || desired.providerId !== "hetzner"
    || [
      retained.providerServerId,
      retained.runtimeMachineId,
      retained.runtimeKernelId,
      retained.runtimeRelayRealmId,
      retained.runtimeReleaseDigest,
      desired.providerImageId,
      desired.providerProfileId,
      desired.providerProfileDigest,
      desired.runtimeReleaseDigest,
      desired.runtimeSourceCommit,
      desired.runtimeSourceTree,
    ].some((value) => value.trim() === "")) {
    throw new Error("The server-authoritative managed reimage preflight is incomplete or unconverged.")
  }
  return preflight
}

function validateObservation(
  acknowledgement: ManagedEnvironmentPreReimageObservationAcknowledgement,
  preflight: ManagedEnvironmentReimagePreflight,
): void {
  if (acknowledgement.environmentId !== preflight.environmentId
    || acknowledgement.generation !== preflight.retained.generation
    || acknowledgement.observedAt.trim() === "") {
    throw new Error("The old kernel returned a mismatched pre-reimage observation acknowledgement.")
  }
}

function requestFromPreflight(
  preflight: ManagedEnvironmentReimagePreflight,
  contextPlan: ManagedEnvironmentContextPlanInput,
  idempotencyKey: string,
): ImmutableReimageRequest {
  if (!idempotencyKey.trim()) throw new Error("Managed reimage requires an idempotency key.")
  return {
    environmentId: preflight.environmentId,
    expectedGeneration: preflight.retained.generation,
    expectedProviderServerId: preflight.retained.providerServerId,
    expectedProviderImageId: preflight.desiredRelease.providerImageId,
    expectedProviderProfileId: preflight.desiredRelease.providerProfileId,
    expectedProviderProfileDigest: preflight.desiredRelease.providerProfileDigest,
    expectedRuntimeReleaseDigest: preflight.desiredRelease.runtimeReleaseDigest,
    expectedRuntimeSourceCommit: preflight.desiredRelease.runtimeSourceCommit,
    expectedRuntimeSourceTree: preflight.desiredRelease.runtimeSourceTree,
    contextPlan,
    idempotencyKey,
  }
}

function confirmationFromPreflight(
  preflight: ManagedEnvironmentReimagePreflight,
): ManagedEnvironmentReimageConfirmation {
  return {
    environmentId: preflight.environmentId,
    providerServerId: preflight.retained.providerServerId,
    generation: preflight.retained.generation,
    runtimeMachineId: preflight.retained.runtimeMachineId,
    runtimeKernelId: preflight.retained.runtimeKernelId,
    providerImageId: preflight.desiredRelease.providerImageId,
    providerProfileId: preflight.desiredRelease.providerProfileId,
    providerProfileDigest: preflight.desiredRelease.providerProfileDigest,
    runtimeReleaseDigest: preflight.desiredRelease.runtimeReleaseDigest,
    runtimeSourceCommit: preflight.desiredRelease.runtimeSourceCommit,
    runtimeSourceTree: preflight.desiredRelease.runtimeSourceTree,
  }
}

function validateReimageResult(
  result: ManagedEnvironmentReimageResult,
  pending: PendingReimage,
): ManagedEnvironmentReimageResult {
  const request = pending.request
  const receipt = result.receipt
  if (result.environment.environmentId !== request.environmentId
    || result.operation.environmentId !== request.environmentId
    || result.operation.kind !== "reimage"
    || result.operation.idempotencyKey !== request.idempotencyKey
    || receipt.environmentId !== request.environmentId
    || receipt.operationId !== result.operation.operationId
    || receipt.previousGeneration !== request.expectedGeneration
    || receipt.generation !== request.expectedGeneration + 1
    || receipt.providerServerId !== request.expectedProviderServerId
    || receipt.providerImageId !== request.expectedProviderImageId
    || receipt.providerProfileId !== request.expectedProviderProfileId
    || receipt.providerProfileDigest !== request.expectedProviderProfileDigest
    || receipt.runtimeReleaseDigest !== request.expectedRuntimeReleaseDigest) {
    throw new Error("Managed reimage polling returned evidence for a different immutable request.")
  }
  return result
}

function validateReadyReplacement(
  environment: ManagedEnvironmentSummary,
  result: ManagedEnvironmentReimageResult,
  pending: PendingReimage,
): ManagedEnvironmentSummary {
  const receipt = result.receipt
  const old = pending.preflight.retained
  if (!receipt.newMachineId?.trim()
    || !receipt.newKernelId?.trim()
    || receipt.newMachineId === old.runtimeMachineId
    || receipt.newKernelId === old.runtimeKernelId
    || (receipt.oldMachineId !== null && receipt.oldMachineId !== old.runtimeMachineId)
    || (receipt.oldKernelId !== null && receipt.oldKernelId !== old.runtimeKernelId)
    || environment.environmentId !== pending.request.environmentId
    || environment.runtimeMachineId !== receipt.newMachineId
    || environment.runtimeKernelId !== receipt.newKernelId
    || environment.runtimeReleaseDigest !== pending.request.expectedRuntimeReleaseDigest
    || !replacementIsReadyForContextOrLaunch(environment)
    || !contextPlanMatchesEnvironment(pending.request.contextPlan, environment)) {
    throw new Error("Managed reimage returned a stale or mismatched replacement identity.")
  }
  return environment
}

function replacementIsReadyForContextOrLaunch(environment: ManagedEnvironmentSummary): boolean {
  if (environment.desiredState !== "running"
    || !environment.runtimeMachineId
    || !environment.runtimeKernelId) {
    return false
  }
  if (environment.observedState === "awaiting_context") return true
  return environment.observedState === "ready"
    && environment.observedRevision === environment.desiredRevision
    && Boolean(environment.contextManifestDigest)
}

function contextPlanMatchesEnvironment(
  expected: ManagedEnvironmentContextPlanInput,
  environment: ManagedEnvironmentSummary,
): boolean {
  const actual: ManagedEnvironmentContextPlanInput = {
    sourceTargetId: environment.contextPlan.source?.sourceTargetId ?? null,
    kernelContext: environment.contextPlan.kernelContext,
    developmentSetup: environment.contextPlan.developmentSetup,
    providerAccounts: environment.contextPlan.providerAccounts,
    gitCredentials: environment.contextPlan.gitCredentials,
  }
  return contextPlanMatchesInput(expected, actual)
}

function contextPlanMatchesInput(
  left: ManagedEnvironmentContextPlanInput,
  right: ManagedEnvironmentContextPlanInput,
): boolean {
  return left.sourceTargetId === right.sourceTargetId
    && left.kernelContext === right.kernelContext
    && developmentSetupMatches(left.developmentSetup, right.developmentSetup)
    && providerAccountsMatch(left.providerAccounts, right.providerAccounts)
    && gitCredentialsMatch(left.gitCredentials, right.gitCredentials)
}

function developmentSetupMatches(
  left: ManagedEnvironmentContextPlanInput["developmentSetup"],
  right: ManagedEnvironmentContextPlanInput["developmentSetup"],
): boolean {
  if (left.kind !== right.kind) return false
  if (left.kind === "empty" || right.kind === "empty") return true
  return left.projectId === right.projectId
    && left.repositories.length === right.repositories.length
    && left.repositories.every((repository, index) => {
      const candidate = right.repositories[index]
      return repository.role === candidate?.role
        && repository.workspaceId === candidate.workspaceId
        && repository.worktreeId === candidate.worktreeId
    })
}

function providerAccountsMatch(
  left: ManagedEnvironmentContextPlanInput["providerAccounts"],
  right: ManagedEnvironmentContextPlanInput["providerAccounts"],
): boolean {
  if (left.kind !== right.kind) return false
  if (left.kind === "none" || right.kind === "none") return true
  const orderedLeft = [...left.accounts].sort(compareProviderAccounts)
  const orderedRight = [...right.accounts].sort(compareProviderAccounts)
  return orderedLeft.length === orderedRight.length
    && orderedLeft.every((account, index) => (
      account.provider === orderedRight[index]?.provider
      && account.accountProfile === orderedRight[index]?.accountProfile
    ))
}

function compareProviderAccounts(
  left: { readonly provider: string; readonly accountProfile: string },
  right: { readonly provider: string; readonly accountProfile: string },
): number {
  return compareStrings(left.provider, right.provider)
    || compareStrings(left.accountProfile, right.accountProfile)
}

function gitCredentialsMatch(
  left: ManagedEnvironmentContextPlanInput["gitCredentials"],
  right: ManagedEnvironmentContextPlanInput["gitCredentials"],
): boolean {
  if (left.kind !== right.kind) return false
  if (left.kind === "none" || right.kind === "none") return true
  const orderedLeft = [...left.credentialIds].sort(compareStrings)
  const orderedRight = [...right.credentialIds].sort(compareStrings)
  return orderedLeft.length === orderedRight.length
    && orderedLeft.every((credentialId, index) => credentialId === orderedRight[index])
}

function compareStrings(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0
}
