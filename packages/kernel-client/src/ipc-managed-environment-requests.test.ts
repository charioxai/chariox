import assert from "node:assert/strict"
import test from "node:test"

import {
  createManagedEnvironmentRequest,
  getManagedEnvironmentRequest,
  listManagedEnvironmentCatalogRequest,
  requestManagedEnvironmentLifecycleRequest,
} from "./ipc-managed-environment-requests.js"

test("managed environment requests use the shared local daemon shape", () => {
  assert.deepEqual(listManagedEnvironmentCatalogRequest(), { ListManagedEnvironmentCatalog: null })
  assert.deepEqual(getManagedEnvironmentRequest("environment-1"), {
    GetManagedEnvironment: { environmentId: "environment-1" },
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
})
