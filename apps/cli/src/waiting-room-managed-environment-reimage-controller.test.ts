import assert from "node:assert/strict"
import test from "node:test"

import type {
  ManagedEnvironmentContextPlanInput,
  ManagedEnvironmentReimagePreflight,
  ManagedEnvironmentReimageResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import {
  WaitingRoomManagedEnvironmentReimageController,
  type ManagedEnvironmentReimageAttempt,
  type WaitingRoomManagedEnvironmentReimageControllerDeps,
} from "./waiting-room-managed-environment-reimage-controller.js"

test("reimage refuses an unconverged preflight before observing or mutating", async () => {
  const harness = reimageHarness()
  harness.preflight = preflight({ retained: { desiredRevision: 9, observedRevision: 8 } })

  await assert.rejects(
    harness.controller.prepare("environment-1", emptyContextInput(), harness.attempt),
    /incomplete or unconverged/,
  )

  assert.equal(harness.observations, 0)
  assert.equal(harness.requests.length, 0)
})

test("reimage refuses a mismatched old-kernel observation before confirmation", async () => {
  const harness = reimageHarness()
  harness.observationGeneration = 8

  await assert.rejects(
    harness.controller.prepare("environment-1", emptyContextInput(), harness.attempt),
    /mismatched pre-reimage observation/,
  )

  assert.equal(harness.observations, 1)
  assert.equal(harness.requests.length, 0)
  await assert.rejects(
    harness.controller.confirm("environment-1", harness.attempt),
    /Prepare this managed reimage/,
  )
})

test("reimage cancellation is mutation-free before explicit confirmation", async () => {
  const harness = reimageHarness()

  const prepared = await harness.controller.prepare(
    "environment-1",
    emptyContextInput(),
    harness.attempt,
  )
  assert.equal(prepared.status, "confirmation_required")
  assert.deepEqual(harness.controller.cancel("environment-1"), { status: "cancelled" })
  assert.equal(harness.requests.length, 0)
  assert.equal(harness.launches, 0)
  await assert.rejects(
    harness.controller.confirm("environment-1", harness.attempt),
    /Prepare this managed reimage/,
  )
})

test("reimage snapshots caller context and authoritative preflight before old-kernel observation", async () => {
  const harness = reimageHarness()
  const callerContext = sourceContextInput()
  const authorizedContext = structuredClone(callerContext)
  harness.environment = readyEnvironment({ contextPlan: contextPlanFromInput(authorizedContext) })
  const servicePreflight = harness.preflight
  const observation = deferred<void>()
  harness.observationGate = observation.promise

  const preparing = harness.controller.prepare(
    "environment-1",
    callerContext,
    harness.attempt,
  )
  await waitUntil(() => harness.observations === 1)

  ;(callerContext as { sourceTargetId: string | null }).sourceTargetId = "source-mutated"
  ;(callerContext.providerAccounts as {
    kind: "selected"
    accounts: Array<{ provider: string; accountProfile: string }>
  }).accounts[0]!.accountProfile = "mutated"
  ;(servicePreflight.retained as { providerServerId: string }).providerServerId = "server-mutated"
  ;(servicePreflight.desiredRelease as { providerImageId: string }).providerImageId = "image-mutated"
  observation.resolve()

  const prepared = await preparing
  assert.equal(prepared.confirmation.providerServerId, "server-1")
  assert.equal(prepared.confirmation.providerImageId, "image-2")
  await harness.controller.confirm("environment-1", harness.attempt)

  assert.deepEqual(harness.serializedRequests, [{
    environmentId: "environment-1",
    expectedGeneration: 3,
    expectedProviderServerId: "server-1",
    expectedProviderImageId: "image-2",
    expectedProviderProfileId: "path1",
    expectedProviderProfileDigest: "sha256:profile",
    expectedRuntimeReleaseDigest: "sha256:new-release",
    expectedRuntimeSourceCommit: "1".repeat(40),
    expectedRuntimeSourceTree: "2".repeat(40),
    contextPlan: authorizedContext,
    idempotencyKey: "reimage-key-1",
  }])
})

test("reimage keeps the same serialized context after prepare and across a lost-response retry", async () => {
  const harness = reimageHarness()
  const callerContext = sourceContextInput()
  const authorizedContext = structuredClone(callerContext)
  harness.environment = readyEnvironment({ contextPlan: contextPlanFromInput(authorizedContext) })
  harness.requestFailures = 1
  await harness.controller.prepare("environment-1", callerContext, harness.attempt)

  ;(callerContext as { kernelContext: "empty" | "source_kernel" }).kernelContext = "empty"
  ;(callerContext.developmentSetup as {
    kind: "source_project"
    projectId: string
  }).projectId = "project-mutated-before-request"
  await assert.rejects(
    harness.controller.confirm("environment-1", harness.attempt),
    /response was lost/,
  )

  ;(callerContext as { sourceTargetId: string | null }).sourceTargetId = "source-mutated-before-retry"
  ;(callerContext.gitCredentials as {
    kind: "selected"
    credentialIds: string[]
  }).credentialIds.push("credential-mutated")
  await harness.controller.confirm("environment-1", harness.attempt)

  assert.equal(harness.serializedRequests.length, 2)
  assert.deepEqual(harness.serializedRequests[0]?.contextPlan, authorizedContext)
  assert.deepEqual(harness.serializedRequests[1]?.contextPlan, authorizedContext)
  assert.deepEqual(harness.serializedRequests[1], harness.serializedRequests[0])
})

test("repeated preparation refuses a changed context instead of reusing the pending request", async () => {
  const harness = reimageHarness()
  await harness.controller.prepare("environment-1", sourceContextInput(), harness.attempt)
  const changed = sourceContextInput()
  ;(changed as { sourceTargetId: string | null }).sourceTargetId = "source-2"

  await assert.rejects(
    harness.controller.prepare("environment-1", changed, harness.attempt),
    /selected reimage context changed/,
  )

  assert.equal(harness.observations, 1)
  assert.equal(harness.requests.length, 0)
})

test("accepted reimage retries the exact immutable request and cannot pretend to cancel", async () => {
  const harness = reimageHarness()
  harness.requestFailures = 1
  await harness.controller.prepare("environment-1", emptyContextInput(), harness.attempt)

  await assert.rejects(
    harness.controller.confirm("environment-1", harness.attempt),
    /response was lost/,
  )
  const cancellation = harness.controller.cancel("environment-1")
  assert.equal(cancellation.status, "irreversible")

  const replacement = await harness.controller.confirm("environment-1", harness.attempt)
  assert.equal(replacement.runtimeKernelId, "kernel-new")
  assert.equal(harness.requests.length, 2)
  assert.deepEqual(harness.requests[1], harness.requests[0])
  assert.equal(harness.requests[0]?.idempotencyKey, "reimage-key-1")
  assert.equal(harness.launches, 1)
  assert.deepEqual(harness.controller.cancel("environment-1"), { status: "nothing_pending" })
})

test("reimage rejects a stale replacement identity and retains resumable state", async () => {
  const harness = reimageHarness()
  harness.environment = readyEnvironment({
    runtimeMachineId: "machine-old",
    runtimeKernelId: "kernel-old",
  })
  harness.result = freshResult({
    receipt: { newMachineId: "machine-old", newKernelId: "kernel-old" },
  })
  await harness.controller.prepare("environment-1", emptyContextInput(), harness.attempt)

  await assert.rejects(
    harness.controller.confirm("environment-1", harness.attempt),
    /stale or mismatched replacement identity/,
  )

  assert.equal(harness.launches, 0)
  assert.equal(harness.controller.cancel("environment-1").status, "irreversible")
})

test("reimage completes only after replacement pivot and Project readiness gate", async () => {
  const harness = reimageHarness()
  let releaseLaunch!: () => void
  const launchGate = new Promise<void>((resolve) => { releaseLaunch = resolve })
  harness.launchGate = launchGate
  await harness.controller.prepare("environment-1", emptyContextInput(), harness.attempt)

  let completed = false
  const confirmation = harness.controller.confirm("environment-1", harness.attempt).then((value) => {
    completed = true
    return value
  })
  const duplicateConfirmation = harness.controller.confirm("environment-1", harness.attempt)
  await new Promise<void>((resolve) => setImmediate(resolve))

  assert.equal(harness.launches, 1)
  assert.equal(harness.requests.length, 1)
  assert.equal(completed, false)
  assert.equal(harness.controller.cancel("environment-1").status, "irreversible")
  releaseLaunch()
  const [replacement, duplicateReplacement] = await Promise.all([
    confirmation,
    duplicateConfirmation,
  ])
  assert.equal(replacement.runtimeMachineId, "machine-new")
  assert.equal(duplicateReplacement.runtimeMachineId, "machine-new")
  assert.equal(completed, true)
})

function reimageHarness() {
  type ReimageRequest = Parameters<
    WaitingRoomManagedEnvironmentReimageControllerDeps["requestReimage"]
  >[0]
  const requests: ReimageRequest[] = []
  const serializedRequests: ReimageRequest[] = []
  const progress: string[] = []
  const state = {
    preflight: preflight(),
    observationGeneration: 3,
    observations: 0,
    requestFailures: 0,
    result: freshResult(),
    environment: readyEnvironment(),
    launches: 0,
    launchGate: Promise.resolve(),
    observationGate: Promise.resolve(),
  }
  const attempt: ManagedEnvironmentReimageAttempt = {
    assertActive: () => {},
    progress: (message) => { progress.push(message) },
  }
  const deps: WaitingRoomManagedEnvironmentReimageControllerDeps = {
    getPreflight: async () => state.preflight,
    observePreviousKernel: async () => {
      state.observations += 1
      await state.observationGate
      return {
        environmentId: "environment-1",
        generation: state.observationGeneration,
        observedAt: "2026-09-22T00:00:00.000Z",
      }
    },
    requestReimage: async (input) => {
      requests.push(input)
      serializedRequests.push(structuredClone(input))
      if (state.requestFailures > 0) {
        state.requestFailures -= 1
        throw new Error("response was lost")
      }
      return state.result
    },
    getEnvironment: async () => state.environment,
    launchReplacement: async () => {
      state.launches += 1
      await state.launchGate
    },
    createIdempotencyKey: () => "reimage-key-1",
    delay: async () => {},
    nowMs: () => 1,
  }
  const controller = new WaitingRoomManagedEnvironmentReimageController(deps)
  return Object.assign(state, {
    controller,
    attempt,
    requests,
    serializedRequests,
    progress,
  })
}

function preflight(overrides: {
  retained?: Partial<ManagedEnvironmentReimagePreflight["retained"]>
  desiredRelease?: Partial<ManagedEnvironmentReimagePreflight["desiredRelease"]>
} = {}): ManagedEnvironmentReimagePreflight {
  return {
    environmentId: "environment-1",
    retained: {
      providerServerId: "server-1",
      generation: 3,
      desiredRevision: 7,
      observedRevision: 7,
      runtimeMachineId: "machine-old",
      runtimeKernelId: "kernel-old",
      runtimeRelayRealmId: "realm-old",
      runtimeReleaseDigest: "sha256:old-release",
      ...overrides.retained,
    },
    desiredRelease: {
      providerId: "hetzner",
      providerImageId: "image-2",
      providerProfileId: "path1",
      providerProfileDigest: "sha256:profile",
      runtimeReleaseDigest: "sha256:new-release",
      runtimeSourceCommit: "1".repeat(40),
      runtimeSourceTree: "2".repeat(40),
      ...overrides.desiredRelease,
    },
  }
}

function emptyContextInput(): ManagedEnvironmentContextPlanInput {
  return {
    sourceTargetId: null,
    kernelContext: "empty",
    developmentSetup: { kind: "empty" },
    providerAccounts: { kind: "none" },
    gitCredentials: { kind: "none" },
  }
}

function sourceContextInput(): ManagedEnvironmentContextPlanInput {
  return {
    sourceTargetId: "source-1",
    kernelContext: "source_kernel",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [{ role: "primary", workspaceId: "workspace-1", worktreeId: null }],
    },
    providerAccounts: {
      kind: "selected",
      accounts: [{ provider: "codex", accountProfile: "default" }],
    },
    gitCredentials: { kind: "selected", credentialIds: ["github"] },
  }
}

function contextPlanFromInput(
  input: ManagedEnvironmentContextPlanInput,
): ManagedEnvironmentSummary["contextPlan"] {
  return {
    schemaVersion: 1,
    contextId: "context-new",
    planDigest: "sha256:context",
    source: input.sourceTargetId
      ? {
          sourceTargetId: input.sourceTargetId,
          relayRealmId: "realm-source",
          machineId: "machine-source",
          kernelId: "kernel-source",
          keyThumbprint: "sha256:source-key",
        }
      : null,
    kernelContext: input.kernelContext,
    developmentSetup: input.developmentSetup,
    providerAccounts: input.providerAccounts,
    gitCredentials: input.gitCredentials,
  }
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  const promise = new Promise<T>((nextResolve) => { resolve = nextResolve })
  return { promise, resolve }
}

async function waitUntil(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return
    await new Promise<void>((resolve) => setImmediate(resolve))
  }
  throw new Error("timed out waiting for the deterministic reimage test boundary")
}

function readyEnvironment(overrides: Partial<ManagedEnvironmentSummary> = {}): ManagedEnvironmentSummary {
  return {
    environmentId: "environment-1",
    accountId: "account-1",
    createdByUserId: "user-1",
    name: "Managed agent",
    region: "hel1",
    computeClass: "agent-small",
    desiredState: "running",
    observedState: "ready",
    desiredRevision: 8,
    observedRevision: 8,
    runtimeMachineId: "machine-new",
    runtimeKernelId: "kernel-new",
    runtimeReleaseDigest: "sha256:new-release",
    contextPlan: {
      schemaVersion: 1,
      contextId: "context-new",
      planDigest: "sha256:context",
      source: null,
      kernelContext: "empty",
      developmentSetup: { kind: "empty" },
      providerAccounts: { kind: "none" },
      gitCredentials: { kind: "none" },
    },
    contextManifestDigest: "sha256:manifest",
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    lastErrorCode: null,
    lastErrorMessage: null,
    createdAt: "2026-09-22T00:00:00.000Z",
    updatedAt: "2026-09-22T00:01:00.000Z",
    ...overrides,
  }
}

function freshResult(overrides: {
  receipt?: Partial<ManagedEnvironmentReimageResult["receipt"]>
} = {}): ManagedEnvironmentReimageResult {
  const environment = readyEnvironment()
  return {
    environment,
    operation: {
      operationId: "operation-1",
      environmentId: "environment-1",
      requestedByUserId: "user-1",
      kind: "reimage",
      idempotencyKey: "reimage-key-1",
      requestDigest: "sha256:request",
      desiredRevision: 8,
      status: "succeeded",
      attempt: 1,
      retryable: false,
      failureCode: null,
      failureMessage: null,
      completedAt: "2026-09-22T00:01:00.000Z",
      createdAt: "2026-09-22T00:00:00.000Z",
      updatedAt: "2026-09-22T00:01:00.000Z",
    },
    receipt: {
      receiptId: "receipt-1",
      environmentId: "environment-1",
      operationId: "operation-1",
      previousGeneration: 3,
      generation: 4,
      status: "fresh_equivalent",
      freshEquivalent: true,
      providerServerId: "server-1",
      previousProviderImageId: "image-1",
      providerImageId: "image-2",
      providerProfileId: "path1",
      providerProfileDigest: "sha256:profile",
      runtimeReleaseDigest: "sha256:new-release",
      oldMachineId: "machine-old",
      newMachineId: "machine-new",
      oldKernelId: "kernel-old",
      newKernelId: "kernel-new",
      oldRelayRealmId: "realm-old",
      newRelayRealmId: "realm-new",
      oldRelayTargetId: "target-old",
      newRelayTargetId: "target-new",
      oldBootstrapGrantId: "grant-old",
      newBootstrapGrantId: "grant-new",
      oldCredentialIds: [],
      newCredentialIds: [],
      runtimeEvidence: {},
      sourceEvidence: {
        providerImageId: "image-2",
        providerProfileId: "path1",
        providerProfileDigest: "sha256:profile",
        runtimeReleaseDigest: "sha256:new-release",
        runtimeSourceCommit: "1".repeat(40),
        runtimeSourceTree: "2".repeat(40),
      },
      residueChecks: {},
      revocations: {},
      billingObservation: {},
      resourceObservation: {},
      cleanupState: {},
      rollbackState: {},
      receiptDigest: "sha256:receipt",
      failureCode: null,
      failureMessage: null,
      requestedAt: "2026-09-22T00:00:00.000Z",
      completedAt: "2026-09-22T00:01:00.000Z",
      createdAt: "2026-09-22T00:00:00.000Z",
      updatedAt: "2026-09-22T00:01:00.000Z",
      ...overrides.receipt,
    },
  }
}
