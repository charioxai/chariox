import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcError } from "./ipc.js"
import type {
  ManagedContextLaunchTarget,
  ManagedContextTransferStatus,
} from "@chariox/kernel-client/ipc-managed-context-requests"
import type {
  ManagedContextTransferTicket,
  ManagedEnvironmentContextPlan,
  ManagedEnvironmentResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import {
  WaitingRoomManagedEnvironmentLaunchController,
} from "./waiting-room-managed-environment-launch-controller.js"

test("managed TUI launch creates Empty context and returns the target workspace", async () => {
  const harness = createHarness({
    createResults: [result(environment("provisioning"))],
    getResults: [environment("ready")],
  })

  const prepared = await harness.controller.prepare(newSelection(emptyPlan()), harness.attempt)

  assert.equal(harness.createdInputs.length, 1)
  assert.equal(harness.createdInputs[0]?.clientRequestId, "key-1")
  assert.deepEqual(harness.connectedKernels, [{ machineId: "machine-managed", kernelId: "kernel-managed" }])
  assert.equal(prepared.workspacePath, "/managed/empty/context-1")
  assert.equal(prepared.worktreePath, "/managed/empty/context-1")
  await prepared.commit()
  assert.equal(harness.connectionCommits, 1)
  assert.equal(harness.connectionRollbacks, 0)
})

test("managed TUI launch starts and monitors direct source transfer", async () => {
  const plan = sourcePlan()
  const harness = createHarness({
    createResults: [result(environment("awaiting_context", { contextPlan: plan }))],
    getResults: [
      environment("awaiting_context", { contextPlan: plan }),
      environment("ready", { contextPlan: plan }),
    ],
    startedTransfer: transferStatus("uploading", plan),
    transferStatuses: [transferStatus("completed", plan)],
    launchTarget: sourceLaunchTarget(),
  })

  const prepared = await harness.controller.prepare(newSelection(plan), harness.attempt)

  assert.deepEqual(harness.transferStarts, ["context-1"])
  assert.deepEqual(harness.transferStatusRequests, ["context-1"])
  assert.equal(prepared.workspacePath, "/managed/context-1/primary")
  assert.equal(prepared.launchTarget.development.kind, "from_source")
})

test("managed TUI launch starts a converged stopped environment", async () => {
  const stopped = environment("stopped", {
    desiredState: "stopped",
    desiredRevision: 2,
    observedRevision: 2,
  })
  const harness = createHarness({
    existing: stopped,
    lifecycleResults: [result(environment("starting", {
      desiredRevision: 3,
      observedRevision: 2,
    }))],
    getResults: [environment("ready", { desiredRevision: 3, observedRevision: 3 })],
  })

  await harness.controller.prepare({ kind: "existing", environmentId: "environment-1" }, harness.attempt)

  assert.deepEqual(harness.lifecycleRequests, [{
    environmentId: "environment-1",
    action: "start",
    idempotencyKey: "key-1",
  }])
})

test("managed TUI launch retries the READY to local-Consumed target gap", async () => {
  const harness = createHarness({
    createResults: [result(environment("ready"))],
    launchTargetResults: [
      new LocalIpcError(
        "handle kernel response",
        "launch target is not committed yet",
        "managed_context_launch_target_unavailable",
        true,
      ),
      emptyLaunchTarget(),
    ],
  })

  const prepared = await harness.controller.prepare(newSelection(emptyPlan()), harness.attempt)

  assert.equal(prepared.workspacePath, "/managed/empty/context-1")
  assert.equal(harness.launchTargetRequests.length, 2)
})

test("managed TUI launch retries transient target connection failures", async () => {
  const harness = createHarness({
    createResults: [result(environment("ready"))],
    connectResults: [
      new LocalIpcError("connect relay", "target is not present yet", "target_unavailable", true),
      false,
      true,
    ],
  })

  const prepared = await harness.controller.prepare(newSelection(emptyPlan()), harness.attempt)

  assert.equal(prepared.workspacePath, "/managed/empty/context-1")
  assert.equal(harness.connectAttempts, 3)
})

test("managed TUI launch fails closed on mismatched transfer ticket", async () => {
  const plan = sourcePlan()
  const wrongTicket = transferTicket(plan)
  const harness = createHarness({
    createResults: [result(environment("awaiting_context", { contextPlan: plan }))],
    transferTicket: {
      ...wrongTicket,
      target: { ...wrongTicket.target, kernelId: "kernel-other" },
    },
  })

  await assert.rejects(
    harness.controller.prepare(newSelection(plan), harness.attempt),
    /different managed environment binding/,
  )
  assert.deepEqual(harness.transferStarts, [])
  assert.deepEqual(harness.connectedKernels, [])
})

test("managed TUI launch keeps the create idempotency key after a lost response", async () => {
  const harness = createHarness({
    createErrors: [new Error("response lost")],
    createResults: [result(environment("ready"))],
  })

  await assert.rejects(
    harness.controller.prepare(newSelection(emptyPlan()), harness.attempt),
    /response lost/,
  )
  await harness.controller.prepare(newSelection(emptyPlan()), harness.attempt)

  assert.deepEqual(harness.createdInputs.map((input) => input.clientRequestId), ["key-1", "key-1"])
})

test("managed TUI launch stops before pivot after sticky cancellation", async () => {
  let releaseCreate = () => {}
  const createGate = new Promise<void>((resolve) => {
    releaseCreate = resolve
  })
  const harness = createHarness({
    createResults: [result(environment("ready"))],
    createGate,
  })

  const pending = harness.controller.prepare(newSelection(emptyPlan()), harness.attempt)
  await Promise.resolve()
  harness.cancel()
  releaseCreate()

  await assert.rejects(pending, /cancelled/)
  assert.deepEqual(harness.connectedKernels, [])
})

test("managed TUI launch rolls back the kernel pivot when cancellation follows connection", async () => {
  const harness = createHarness({
    createResults: [result(environment("ready"))],
    launchTargetResults: [new LocalIpcError(
      "get launch target",
      "launch target is not committed yet",
      "managed_context_launch_target_unavailable",
      true,
    )],
    cancelOnDelay: 1,
  })

  await assert.rejects(
    harness.controller.prepare(newSelection(emptyPlan()), harness.attempt),
    /cancelled/,
  )

  assert.equal(harness.connectionCommits, 0)
  assert.equal(harness.connectionRollbacks, 1)
})

type HarnessOptions = {
  existing?: ManagedEnvironmentSummary
  createResults?: ManagedEnvironmentResult[]
  createErrors?: Error[]
  createGate?: Promise<void>
  getResults?: ManagedEnvironmentSummary[]
  lifecycleResults?: ManagedEnvironmentResult[]
  transferTicket?: ManagedContextTransferTicket
  startedTransfer?: ManagedContextTransferStatus
  transferStatuses?: ManagedContextTransferStatus[]
  launchTarget?: ManagedContextLaunchTarget
  launchTargetResults?: Array<ManagedContextLaunchTarget | Error>
  connectResults?: Array<boolean | Error>
  cancelOnDelay?: number
}

function createHarness(options: HarnessOptions) {
  let active = true
  let now = 0
  let keySequence = 0
  let existing = options.existing
  const createdInputs: Array<{ clientRequestId: string }> = []
  const lifecycleRequests: Array<{
    environmentId: string
    action: "start"
    idempotencyKey: string
  }> = []
  const connectedKernels: Array<{ machineId: string; kernelId: string }> = []
  const transferStarts: string[] = []
  const transferStatusRequests: string[] = []
  const launchTargetRequests: Array<{ contextId: string; planDigest: string }> = []
  const createResults = [...(options.createResults ?? [])]
  const createErrors = [...(options.createErrors ?? [])]
  const getResults = [...(options.getResults ?? [])]
  const lifecycleResults = [...(options.lifecycleResults ?? [])]
  const transferStatuses = [...(options.transferStatuses ?? [])]
  const launchTargetResults = [...(options.launchTargetResults ?? [])]
  const connectResults = [...(options.connectResults ?? [])]
  let connectAttempts = 0
  let connectionCommits = 0
  let connectionRollbacks = 0
  let delayCount = 0
  const attempt = {
    assertActive() {
      if (!active) throw new Error("managed launch cancelled")
    },
    environmentChanged() {},
    progress() {},
  }
  const controller = new WaitingRoomManagedEnvironmentLaunchController({
    createEnvironment: async (input) => {
      createdInputs.push(input)
      const error = createErrors.shift()
      if (error) throw error
      await options.createGate
      const next = createResults.shift()
      if (!next) throw new Error("missing create result")
      return next
    },
    getEnvironment: async () => {
      if (existing) {
        const result = existing
        existing = undefined
        return result
      }
      const next = getResults.shift()
      if (!next) throw new Error("missing get result")
      return next
    },
    requestLifecycle: async (input) => {
      lifecycleRequests.push(input)
      const next = lifecycleResults.shift()
      if (!next) throw new Error("missing lifecycle result")
      return next
    },
    prepareContextTransfer: async () => options.transferTicket ?? transferTicket(sourcePlan()),
    startContextTransfer: async (ticket) => {
      transferStarts.push(ticket.contextPlan.contextId)
      return options.startedTransfer ?? transferStatus("completed", ticket.contextPlan)
    },
    getContextTransferStatus: async (contextId) => {
      transferStatusRequests.push(contextId)
      const next = transferStatuses.shift()
      if (!next) throw new Error("missing transfer status")
      return next
    },
    connectKernel: async (machineId, kernelId, isActive) => {
      connectAttempts += 1
      if (!isActive()) return null
      const result = connectResults.shift()
      if (result instanceof Error) throw result
      if (result === false) return null
      connectedKernels.push({ machineId, kernelId })
      return {
        commit: async () => {
          connectionCommits += 1
        },
        rollback: async () => {
          connectionRollbacks += 1
        },
      }
    },
    getLaunchTarget: async (contextId, planDigest) => {
      launchTargetRequests.push({ contextId, planDigest })
      const next = launchTargetResults.shift()
      if (next instanceof Error) throw next
      return next ?? options.launchTarget ?? emptyLaunchTarget()
    },
    createIdempotencyKey: () => `key-${++keySequence}`,
    environmentName: () => "Managed agent",
    delay: async (ms) => {
      now += ms
      delayCount += 1
      if (delayCount === options.cancelOnDelay) active = false
    },
    nowMs: () => now,
  })
  return {
    controller,
    attempt,
    createdInputs,
    lifecycleRequests,
    connectedKernels,
    transferStarts,
    transferStatusRequests,
    launchTargetRequests,
    get connectAttempts() {
      return connectAttempts
    },
    get connectionCommits() {
      return connectionCommits
    },
    get connectionRollbacks() {
      return connectionRollbacks
    },
    cancel: () => {
      active = false
    },
  }
}

function newSelection(contextPlan: ManagedEnvironmentContextPlan) {
  return {
    kind: "new" as const,
    computeClass: "agent-small",
    region: "hel1",
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    contextPlan: {
      sourceTargetId: contextPlan.source?.sourceTargetId ?? null,
      kernelContext: contextPlan.kernelContext,
      developmentSetup: contextPlan.developmentSetup,
      providerAccounts: contextPlan.providerAccounts,
      gitCredentials: contextPlan.gitCredentials,
    },
  }
}

function environment(
  observedState: ManagedEnvironmentSummary["observedState"],
  update: Partial<ManagedEnvironmentSummary> = {},
): ManagedEnvironmentSummary {
  const runtimeBound = observedState === "awaiting_context" || observedState === "ready"
  return {
    environmentId: "environment-1",
    accountId: "account-1",
    createdByUserId: "user-1",
    name: "Managed agent",
    region: "hel1",
    computeClass: "agent-small",
    desiredState: "running",
    observedState,
    desiredRevision: 1,
    observedRevision: observedState === "ready" ? 1 : 0,
    runtimeMachineId: runtimeBound ? "machine-managed" : null,
    runtimeKernelId: runtimeBound ? "kernel-managed" : null,
    runtimeReleaseDigest: runtimeBound ? "sha256:release" : null,
    contextPlan: emptyPlan(),
    contextManifestDigest: observedState === "ready" ? "sha256:manifest" : null,
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    lastErrorCode: null,
    lastErrorMessage: null,
    createdAt: "2026-08-21T00:00:00.000Z",
    updatedAt: "2026-08-21T00:00:00.000Z",
    ...update,
  }
}

function result(value: ManagedEnvironmentSummary): ManagedEnvironmentResult {
  return {
    environment: value,
    operation: {
      operationId: "operation-1",
      environmentId: value.environmentId,
      requestedByUserId: "user-1",
      kind: "create",
      idempotencyKey: "key-1",
      requestDigest: "sha256:request",
      desiredRevision: value.desiredRevision,
      status: "pending",
      attempt: 0,
      retryable: false,
      failureCode: null,
      failureMessage: null,
      completedAt: null,
      createdAt: "2026-08-21T00:00:00.000Z",
      updatedAt: "2026-08-21T00:00:00.000Z",
    },
  }
}

function emptyPlan(): ManagedEnvironmentContextPlan {
  return {
    schemaVersion: 1,
    contextId: "context-1",
    planDigest: "sha256:plan",
    source: null,
    kernelContext: "empty",
    developmentSetup: { kind: "empty" },
    providerAccounts: { kind: "none" },
    gitCredentials: { kind: "none" },
  }
}

function sourcePlan(): ManagedEnvironmentContextPlan {
  return {
    ...emptyPlan(),
    source: {
      sourceTargetId: "source-target-1",
      relayRealmId: "realm-1",
      machineId: "machine-source",
      kernelId: "kernel-source",
      keyThumbprint: "sha256:source-key",
    },
    kernelContext: "source_kernel",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [
        { role: "primary", workspaceId: "workspace-primary", worktreeId: "worktree-primary" },
        { role: "supporting", workspaceId: "workspace-supporting", worktreeId: null },
      ],
    },
  }
}

function transferTicket(plan: ManagedEnvironmentContextPlan): ManagedContextTransferTicket {
  return {
    environmentId: "environment-1",
    contextPlan: plan,
    target: {
      relayRealmId: "realm-1",
      machineId: "machine-managed",
      kernelId: "kernel-managed",
      relayPublicKey: "public-key",
      keyThumbprint: "sha256:target-key",
    },
  }
}

function transferStatus(
  phase: ManagedContextTransferStatus["phase"],
  plan: ManagedEnvironmentContextPlan,
): ManagedContextTransferStatus {
  return {
    contextId: plan.contextId,
    planDigest: plan.planDigest,
    phase,
    acceptedBytes: phase === "preparing" ? 0 : 10,
    packageSizeBytes: 10,
    retryable: false,
    updatedAtMs: 1,
  }
}

function emptyLaunchTarget(): ManagedContextLaunchTarget {
  return {
    environmentId: "environment-1",
    kernelId: "kernel-managed",
    contextId: "context-1",
    planDigest: "sha256:plan",
    development: { kind: "empty", workspacePath: "/managed/empty/context-1" },
  }
}

function sourceLaunchTarget(): ManagedContextLaunchTarget {
  return {
    ...emptyLaunchTarget(),
    development: {
      kind: "from_source",
      projectId: "project-1",
      destinationRoot: "/managed/context-1",
      primaryRepositoryId: "repository-primary",
      repositories: [
        {
          repositoryId: "repository-primary",
          role: "primary",
          targetDirectory: "primary",
          workspacePath: "/managed/context-1/primary",
          headSha: "a".repeat(40),
        },
        {
          repositoryId: "repository-supporting",
          role: "supporting",
          targetDirectory: "supporting",
          workspacePath: "/managed/context-1/supporting",
          headSha: "b".repeat(40),
        },
      ],
    },
  }
}
