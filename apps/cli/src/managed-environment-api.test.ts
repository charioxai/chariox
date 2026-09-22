import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import {
  createManagedEnvironment,
  getManagedContextLaunchTarget,
  getManagedContextTransferStatus,
  getManagedEnvironment,
  getManagedEnvironmentReimagePreflight,
  listManagedEnvironmentCatalog,
  observeManagedEnvironmentPreReimage,
  prepareManagedEnvironmentContextTransfer,
  requestManagedEnvironmentLifecycle,
  requestManagedEnvironmentReimage,
  startManagedContextTransfer,
} from "./managed-environment-api.js"

test("managed environment API uses only shared LocalDaemon request variants", async () => {
  const requests: unknown[] = []
  const responses = [
    { ManagedEnvironmentCatalog: { catalog: { computeClasses: [], contextSources: [], environments: [] } } },
    { ManagedEnvironment: { environment: { environmentId: "environment-1" } } },
    { ManagedEnvironmentReimagePreflight: { preflight: reimagePreflight() } },
    { ManagedEnvironmentCreated: { result: { environment: { environmentId: "environment-1" }, operation: { environmentId: "environment-1" } } } },
    { ManagedEnvironmentLifecycleRequested: { result: { environment: { environmentId: "environment-1" }, operation: { environmentId: "environment-1" } } } },
    { ManagedEnvironmentReimageRequested: { result: reimageResult() } },
    {
      ManagedEnvironmentPreReimageObserved: {
        acknowledgement: {
          environmentId: "environment-1",
          generation: 3,
          observedAt: "2026-09-22T01:02:03.000Z",
        },
      },
    },
    { ManagedEnvironmentContextTransferPrepared: { ticket: ticket() } },
    { ManagedContextTransferStarted: { status: status("preparing") } },
    { ManagedContextTransferStatus: { status: status("completed") } },
    { ManagedContextLaunchTarget: { target: launchTarget() } },
  ]
  const client = {
    send: async (request: unknown) => {
      requests.push(request)
      return responses.shift()
    },
  } as unknown as LocalIpcClient

  await listManagedEnvironmentCatalog(client)
  await getManagedEnvironment(client, "environment-1")
  await getManagedEnvironmentReimagePreflight(client, "environment-1")
  await createManagedEnvironment(client, {
    clientRequestId: "create-1",
    name: "Managed build",
    region: "hel1",
    computeClass: "agent-small",
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    contextPlan: {
      sourceTargetId: null,
      kernelContext: "empty",
      developmentSetup: { kind: "empty" },
      providerAccounts: { kind: "none" },
      gitCredentials: { kind: "none" },
    },
  })
  await requestManagedEnvironmentLifecycle(client, {
    environmentId: "environment-1",
    action: "start",
    idempotencyKey: "start-1",
  })
  await requestManagedEnvironmentReimage(client, {
    environmentId: "environment-1",
    expectedGeneration: 3,
    expectedProviderServerId: "123456789",
    expectedProviderImageId: "987654321",
    expectedProviderProfileId: "hetzner-path1",
    expectedProviderProfileDigest: `sha256:${"b".repeat(64)}`,
    expectedRuntimeReleaseDigest: `sha256:${"a".repeat(64)}`,
    expectedRuntimeSourceCommit: "c".repeat(40),
    expectedRuntimeSourceTree: "d".repeat(40),
    contextPlan: {
      sourceTargetId: "source-target-1",
      kernelContext: "source_kernel",
      developmentSetup: {
        kind: "source_project",
        projectId: "project-1",
        repositories: [{
          role: "primary",
          workspaceId: "workspace-primary",
          worktreeId: null,
        }],
      },
      providerAccounts: { kind: "none" },
      gitCredentials: { kind: "none" },
    },
    idempotencyKey: "reimage-1",
  })
  await observeManagedEnvironmentPreReimage(client, {
    environmentId: "environment-1",
    expectedGeneration: 3,
  })
  await prepareManagedEnvironmentContextTransfer(client, "environment-1")
  await startManagedContextTransfer(client, ticket())
  await getManagedContextTransferStatus(client, "context-1")
  await getManagedContextLaunchTarget(client, "context-1", "sha256:plan")

  assert.deepEqual(requests, [
    { ListManagedEnvironmentCatalog: null },
    { GetManagedEnvironment: { environmentId: "environment-1" } },
    { GetManagedEnvironmentReimagePreflight: { environmentId: "environment-1" } },
    {
      CreateManagedEnvironment: {
        clientRequestId: "create-1",
        name: "Managed build",
        region: "hel1",
        computeClass: "agent-small",
        autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
        contextPlan: {
          sourceTargetId: null,
          kernelContext: "empty",
          developmentSetup: { kind: "empty" },
          providerAccounts: { kind: "none" },
          gitCredentials: { kind: "none" },
        },
      },
    },
    { RequestManagedEnvironmentLifecycle: { environmentId: "environment-1", action: "start", idempotencyKey: "start-1" } },
    {
      RequestManagedEnvironmentReimage: {
        environmentId: "environment-1",
        expectedGeneration: 3,
        expectedProviderServerId: "123456789",
        expectedProviderImageId: "987654321",
        expectedProviderProfileId: "hetzner-path1",
        expectedProviderProfileDigest: `sha256:${"b".repeat(64)}`,
        expectedRuntimeReleaseDigest: `sha256:${"a".repeat(64)}`,
        expectedRuntimeSourceCommit: "c".repeat(40),
        expectedRuntimeSourceTree: "d".repeat(40),
        contextPlan: {
          sourceTargetId: "source-target-1",
          kernelContext: "source_kernel",
          developmentSetup: {
            kind: "source_project",
            projectId: "project-1",
            repositories: [{
              role: "primary",
              workspaceId: "workspace-primary",
              worktreeId: null,
            }],
          },
          providerAccounts: { kind: "none" },
          gitCredentials: { kind: "none" },
        },
        idempotencyKey: "reimage-1",
      },
    },
    {
      ObserveManagedEnvironmentPreReimage: {
        environmentId: "environment-1",
        expectedGeneration: 3,
      },
    },
    { PrepareManagedEnvironmentContextTransfer: { environmentId: "environment-1" } },
    { StartManagedContextTransfer: { ticket: ticket() } },
    { GetManagedContextTransferStatus: { contextId: "context-1" } },
    { GetManagedContextLaunchTarget: { contextId: "context-1", planDigest: "sha256:plan" } },
  ])
})

test("managed environment API forwards explicit-empty reimage context", async () => {
  const requests: unknown[] = []
  const client = {
    send: async (request: unknown) => {
      requests.push(request)
      return { ManagedEnvironmentReimageRequested: { result: reimageResult() } }
    },
  } as unknown as LocalIpcClient
  const contextPlan = {
    sourceTargetId: null,
    kernelContext: "empty" as const,
    developmentSetup: { kind: "empty" as const },
    providerAccounts: { kind: "none" as const },
    gitCredentials: { kind: "none" as const },
  }

  await requestManagedEnvironmentReimage(client, {
    environmentId: "environment-1",
    expectedGeneration: 3,
    expectedProviderServerId: "123456789",
    expectedProviderImageId: "987654321",
    expectedProviderProfileId: "hetzner-path1",
    expectedProviderProfileDigest: `sha256:${"b".repeat(64)}`,
    expectedRuntimeReleaseDigest: `sha256:${"a".repeat(64)}`,
    expectedRuntimeSourceCommit: "c".repeat(40),
    expectedRuntimeSourceTree: "d".repeat(40),
    contextPlan,
    idempotencyKey: "reimage-1",
  })

  assert.deepEqual(
    (requests[0] as { RequestManagedEnvironmentReimage: { contextPlan: unknown } })
      .RequestManagedEnvironmentReimage.contextPlan,
    contextPlan,
  )
})

test("managed environment API rejects responses for another environment", async () => {
  const client = {
    send: async () => ({ ManagedEnvironment: { environment: { environmentId: "environment-other" } } }),
  } as unknown as LocalIpcClient

  await assert.rejects(
    getManagedEnvironment(client, "environment-1"),
    /different managed environment/,
  )
})

test("managed environment API rejects reimage preflight for another environment", async () => {
  const client = {
    send: async () => ({
      ManagedEnvironmentReimagePreflight: {
        preflight: { ...reimagePreflight(), environmentId: "environment-other" },
      },
    }),
  } as unknown as LocalIpcClient

  await assert.rejects(
    getManagedEnvironmentReimagePreflight(client, "environment-1"),
    /reimage preflight for a different managed environment/,
  )
})

test("managed environment API reports the reimage preflight protocol minimum", async () => {
  const client = {
    send: async () => {
      throw new Error("unknown variant `GetManagedEnvironmentReimagePreflight`")
    },
  } as unknown as LocalIpcClient

  await assert.rejects(
    getManagedEnvironmentReimagePreflight(client, "environment-1"),
    /Managed environment reimage preflight requires kernel protocol 341 or newer/,
  )
})

test("managed environment API rejects stale or incomplete reimage evidence", async () => {
  const result = reimageResult()
  result.receipt.generation = 5
  const client = {
    send: async () => ({ ManagedEnvironmentReimageRequested: { result } }),
  } as unknown as LocalIpcClient

  await assert.rejects(
    requestManagedEnvironmentReimage(client, {
      environmentId: "environment-1",
      expectedGeneration: 3,
      expectedProviderServerId: "123456789",
      expectedProviderImageId: "987654321",
      expectedProviderProfileId: "hetzner-path1",
      expectedProviderProfileDigest: `sha256:${"b".repeat(64)}`,
      expectedRuntimeReleaseDigest: `sha256:${"a".repeat(64)}`,
      expectedRuntimeSourceCommit: "c".repeat(40),
      expectedRuntimeSourceTree: "d".repeat(40),
      contextPlan: {
        sourceTargetId: null,
        kernelContext: "empty",
        developmentSetup: { kind: "empty" },
        providerAccounts: { kind: "none" },
        gitCredentials: { kind: "none" },
      },
      idempotencyKey: "reimage-1",
    }),
    /does not match the requested generation or exact provider identity/,
  )
})

test("managed environment API rejects a mismatched pre-reimage acknowledgement", async () => {
  const client = {
    send: async () => ({
      ManagedEnvironmentPreReimageObserved: {
        acknowledgement: {
          environmentId: "environment-1",
          generation: 4,
          observedAt: "2026-09-22T01:02:03.000Z",
        },
      },
    }),
  } as unknown as LocalIpcClient

  await assert.rejects(
    observeManagedEnvironmentPreReimage(client, {
      environmentId: "environment-1",
      expectedGeneration: 3,
    }),
    /mismatched managed pre-reimage observation acknowledgement/,
  )
})

function ticket() {
  return {
    environmentId: "environment-1",
    contextPlan: {
      schemaVersion: 1 as const,
      contextId: "context-1",
      planDigest: "sha256:plan",
      source: null,
      kernelContext: "empty" as const,
      developmentSetup: { kind: "empty" as const },
      providerAccounts: { kind: "none" as const },
      gitCredentials: { kind: "none" as const },
    },
    target: {
      relayRealmId: "realm-1",
      machineId: "machine-1",
      kernelId: "kernel-1",
      relayPublicKey: "public-key",
      keyThumbprint: "sha256:key",
    },
  }
}

function status(phase: "preparing" | "completed") {
  return {
    contextId: "context-1",
    planDigest: "sha256:plan",
    phase,
    acceptedBytes: 0,
    packageSizeBytes: 0,
    retryable: false,
    updatedAtMs: 1,
  }
}

function launchTarget() {
  return {
    environmentId: "environment-1",
    kernelId: "kernel-1",
    contextId: "context-1",
    planDigest: "sha256:plan",
    development: { kind: "empty" as const, workspacePath: "/managed/workspace" },
  }
}

function reimageResult() {
  return {
    environment: { environmentId: "environment-1" },
    operation: {
      operationId: "operation-reimage-1",
      environmentId: "environment-1",
      kind: "reimage",
      idempotencyKey: "reimage-1",
    },
    receipt: {
      receiptId: "receipt-1",
      environmentId: "environment-1",
      operationId: "operation-reimage-1",
      previousGeneration: 3,
      generation: 4,
      providerServerId: "123456789",
      providerImageId: "987654321",
      providerProfileId: "hetzner-path1",
      providerProfileDigest: `sha256:${"b".repeat(64)}`,
      runtimeReleaseDigest: `sha256:${"a".repeat(64)}`,
      sourceEvidence: {
        providerImageId: "987654321",
        providerProfileId: "hetzner-path1",
        providerProfileDigest: `sha256:${"b".repeat(64)}`,
        runtimeReleaseDigest: `sha256:${"a".repeat(64)}`,
        runtimeSourceCommit: "c".repeat(40),
        runtimeSourceTree: "d".repeat(40),
      },
    },
  }
}

function reimagePreflight() {
  return {
    environmentId: "environment-1",
    retained: {
      providerServerId: "123456789",
      generation: 3,
      desiredRevision: 7,
      observedRevision: 7,
      runtimeMachineId: "managed-machine-3",
      runtimeKernelId: "managed-kernel-3",
      runtimeRelayRealmId: "managed-realm-3",
      runtimeReleaseDigest: `sha256:${"a".repeat(64)}`,
    },
    desiredRelease: {
      providerId: "hetzner" as const,
      providerImageId: "987654321",
      providerProfileId: "hetzner-path1",
      providerProfileDigest: `sha256:${"b".repeat(64)}`,
      runtimeReleaseDigest: `sha256:${"c".repeat(64)}`,
      runtimeSourceCommit: "d".repeat(40),
      runtimeSourceTree: "e".repeat(40),
    },
  }
}
