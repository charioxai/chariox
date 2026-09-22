import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import {
  createManagedEnvironment,
  getManagedContextLaunchTarget,
  getManagedContextTransferStatus,
  getManagedEnvironment,
  listManagedEnvironmentCatalog,
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
    { ManagedEnvironmentCreated: { result: { environment: { environmentId: "environment-1" }, operation: { environmentId: "environment-1" } } } },
    { ManagedEnvironmentLifecycleRequested: { result: { environment: { environmentId: "environment-1" }, operation: { environmentId: "environment-1" } } } },
    { ManagedEnvironmentReimageRequested: { result: reimageResult() } },
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
    idempotencyKey: "reimage-1",
  })
  await prepareManagedEnvironmentContextTransfer(client, "environment-1")
  await startManagedContextTransfer(client, ticket())
  await getManagedContextTransferStatus(client, "context-1")
  await getManagedContextLaunchTarget(client, "context-1", "sha256:plan")

  assert.deepEqual(requests, [
    { ListManagedEnvironmentCatalog: null },
    { GetManagedEnvironment: { environmentId: "environment-1" } },
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
        idempotencyKey: "reimage-1",
      },
    },
    { PrepareManagedEnvironmentContextTransfer: { environmentId: "environment-1" } },
    { StartManagedContextTransfer: { ticket: ticket() } },
    { GetManagedContextTransferStatus: { contextId: "context-1" } },
    { GetManagedContextLaunchTarget: { contextId: "context-1", planDigest: "sha256:plan" } },
  ])
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
      idempotencyKey: "reimage-1",
    }),
    /does not match the requested generation or exact provider identity/,
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
