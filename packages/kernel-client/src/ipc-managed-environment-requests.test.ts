import assert from "node:assert/strict"
import test from "node:test"

import {
  createManagedEnvironmentRequest,
  getManagedEnvironmentReimagePreflightRequest,
  getManagedEnvironmentReimageReceiptRequest,
  getManagedEnvironmentRequest,
  listManagedEnvironmentCatalogRequest,
  managedEnvironmentCreateMinimumProtocolVersion,
  managedEnvironmentReimagePreflightMinimumProtocolVersion,
  managedEnvironmentReimageReceiptMinimumProtocolVersion,
  observeManagedEnvironmentPreReimageRequest,
  prepareManagedEnvironmentContextTransferRequest,
  prepareManagedEnvironmentGitCredentialEnrollmentRequest,
  requestManagedEnvironmentLifecycleRequest,
  requestManagedEnvironmentReimageRequest,
  type ManagedEnvironmentReimagePreflight,
  type ManagedEnvironmentSummary,
} from "./ipc-managed-environment-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("managed environment requests use the shared local daemon shape", () => {
  assert.deepEqual(listManagedEnvironmentCatalogRequest(), { ListManagedEnvironmentCatalog: null })
  assert.deepEqual(getManagedEnvironmentRequest("environment-1"), {
    GetManagedEnvironment: { environmentId: "environment-1" },
  })
  assert.deepEqual(getManagedEnvironmentReimagePreflightRequest("environment-1"), {
    GetManagedEnvironmentReimagePreflight: { environmentId: "environment-1" },
  })
  assert.deepEqual(getManagedEnvironmentReimageReceiptRequest("environment-1"), {
    GetManagedEnvironmentReimageReceipt: { environmentId: "environment-1" },
  })
  assert.equal(managedEnvironmentReimageReceiptMinimumProtocolVersion, 345)
  assert.deepEqual(prepareManagedEnvironmentContextTransferRequest("environment-1"), {
    PrepareManagedEnvironmentContextTransfer: { environmentId: "environment-1" },
  })
  assert.deepEqual(prepareManagedEnvironmentGitCredentialEnrollmentRequest({
    environmentId: "environment-1",
    sourceTargetId: "source-target-1",
    gitCredentials: { kind: "selected", credentialIds: ["github"] },
  }), {
    PrepareManagedEnvironmentGitCredentialEnrollment: {
      environmentId: "environment-1",
      sourceTargetId: "source-target-1",
      gitCredentials: { kind: "selected", credentialIds: ["github"] },
    },
  })
  assert.deepEqual(createManagedEnvironmentRequest({
    clientRequestId: "request-1",
    name: "My machine",
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
  }), {
    CreateManagedEnvironment: {
      clientRequestId: "request-1",
      name: "My machine",
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
  })
  assert.deepEqual(createManagedEnvironmentRequest({
    clientRequestId: "request-root-1",
    name: "Rooted machine",
    region: "hel1",
    computeClass: "agent-small",
    managedRepositoryRoot: "/srv/chariox/repos",
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    contextPlan: {
      sourceTargetId: null,
      kernelContext: "empty",
      developmentSetup: { kind: "empty" },
      providerAccounts: { kind: "none" },
      gitCredentials: { kind: "none" },
    },
  }), {
    CreateManagedEnvironment: {
      clientRequestId: "request-root-1",
      name: "Rooted machine",
      region: "hel1",
      computeClass: "agent-small",
      managedRepositoryRoot: "/srv/chariox/repos",
      autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
      contextPlan: {
        sourceTargetId: null,
        kernelContext: "empty",
        developmentSetup: { kind: "empty" },
        providerAccounts: { kind: "none" },
        gitCredentials: { kind: "none" },
      },
    },
  })
  assert.deepEqual(createManagedEnvironmentRequest({
    clientRequestId: "request-source-1",
    name: "My source machine",
    region: "fsn1",
    computeClass: "agent-medium",
    autoStopPolicy: { minimumRuntimeSeconds: 300, idleDelaySeconds: null },
    contextPlan: {
      sourceTargetId: "source-target-1",
      kernelContext: "source_kernel",
      developmentSetup: {
        kind: "source_project",
        projectId: "project-1",
        repositories: [
          { role: "primary", workspaceId: "workspace-1", worktreeId: "worktree-1" },
          { role: "supporting", workspaceId: "workspace-2", worktreeId: null },
        ],
      },
      providerAccounts: {
        kind: "selected",
        accounts: [{ provider: "codex", accountProfile: "work" }],
      },
      gitCredentials: { kind: "selected", credentialIds: ["github-work"] },
    },
  }), {
    CreateManagedEnvironment: {
      clientRequestId: "request-source-1",
      name: "My source machine",
      region: "fsn1",
      computeClass: "agent-medium",
      autoStopPolicy: { minimumRuntimeSeconds: 300, idleDelaySeconds: null },
      contextPlan: {
        sourceTargetId: "source-target-1",
        kernelContext: "source_kernel",
        developmentSetup: {
          kind: "source_project",
          projectId: "project-1",
          repositories: [
            { role: "primary", workspaceId: "workspace-1", worktreeId: "worktree-1" },
            { role: "supporting", workspaceId: "workspace-2", worktreeId: null },
          ],
        },
        providerAccounts: {
          kind: "selected",
          accounts: [{ provider: "codex", accountProfile: "work" }],
        },
        gitCredentials: { kind: "selected", credentialIds: ["github-work"] },
      },
    },
  })
  for (const action of ["start", "stop", "restart", "delete"] as const) {
    assert.deepEqual(requestManagedEnvironmentLifecycleRequest({
      environmentId: "environment-1",
      action,
      idempotencyKey: `${action}-1`,
    }), {
      RequestManagedEnvironmentLifecycle: {
        environmentId: "environment-1",
        action,
        idempotencyKey: `${action}-1`,
      },
    })
  }
  assert.deepEqual(requestManagedEnvironmentReimageRequest({
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
  }), {
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
        sourceTargetId: null,
        kernelContext: "empty",
        developmentSetup: { kind: "empty" },
        providerAccounts: { kind: "none" },
        gitCredentials: { kind: "none" },
      },
      idempotencyKey: "reimage-1",
    },
  })
  assert.deepEqual(observeManagedEnvironmentPreReimageRequest({
    environmentId: "environment-1",
    expectedGeneration: 3,
  }), {
    ObserveManagedEnvironmentPreReimage: {
      environmentId: "environment-1",
      expectedGeneration: 3,
    },
  })
})

test("managed environment reimage preflight exposes only retained identity and desired release", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 346)
  assert.equal(managedEnvironmentReimagePreflightMinimumProtocolVersion, 341)
  assert.equal(managedEnvironmentCreateMinimumProtocolVersion, 342)
  const preflight: ManagedEnvironmentReimagePreflight = {
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
      providerId: "hetzner",
      providerImageId: "987654321",
      providerProfileId: "hetzner-path1",
      providerProfileDigest: `sha256:${"b".repeat(64)}`,
      runtimeReleaseDigest: `sha256:${"c".repeat(64)}`,
      runtimeSourceCommit: "d".repeat(40),
      runtimeSourceTree: "e".repeat(40),
    },
  }

  assert.equal(preflight.retained.providerServerId, "123456789")
  assert.equal(preflight.desiredRelease.providerImageId, "987654321")
  assert.equal("providerImageId" in preflight.retained, false)
})

test("managed environment summaries bind the runtime machine and kernel", () => {
  const summary: ManagedEnvironmentSummary = {
    environmentId: "environment-1",
    accountId: "account-1",
    createdByUserId: "user-1",
    name: "Managed agent",
    region: "hel1",
    computeClass: "agent-small",
    managedRepositoryRoot: "/home/chariox",
    desiredState: "running",
    observedState: "ready",
    desiredRevision: 1,
    observedRevision: 1,
    runtimeMachineId: "managed-machine-1",
    runtimeKernelId: "managed-kernel-1",
    runtimeReleaseDigest: "sha256:release",
    contextPlan: {
      schemaVersion: 1,
      contextId: "context-1",
      planDigest: "sha256:plan",
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
    createdAt: "2026-08-21T00:00:00.000Z",
    updatedAt: "2026-08-21T00:00:00.000Z",
  }

  assert.equal(summary.runtimeKernelId, "managed-kernel-1")
})
