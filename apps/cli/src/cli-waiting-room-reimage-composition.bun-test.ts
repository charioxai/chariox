import { createCliCommandActionComposition } from "./cli-command-action-composition.js"
import assert from "node:assert/strict"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  ManagedEnvironmentContextPlan,
  ManagedEnvironmentReimagePreflight,
  ManagedEnvironmentReimageResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import type {
  ProjectEnvironmentSetupStartInput,
} from "@chariox/kernel-client/project-environment-setup-orchestration"
import type {
  ProjectEnvironmentSetupStatus,
  RuntimeSession,
} from "@chariox/kernel-client/kernel-types"
import {
  createCliWaitingRoomComposition,
  type CliWaitingRoomCompositionDeps,
} from "./cli-waiting-room-composition.js"
import { CloudClient } from "./cloud-client.js"
import { createHash } from "node:crypto"
import { LocalIpcClient } from "./ipc.js"
import {
  createMutableLocalIpcClient,
  type MutableLocalIpcClient,
} from "./mutable-local-ipc-client.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { managedEnvironmentMachineRef, NEW_MANAGED_MACHINE_REF } from "./waiting-room-managed-environments.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { __setWaitingRoomWorktreeInventoryForTest, resolvePendingWaitingRoomWorktreePath, stageWaitingRoomWorktreeSelection, waitingRoomWorktreeDisabledHint, waitingRoomWorktreeOptions } from "./waiting-room-worktrees.js"

const LOCAL_ENDPOINT = "ws://local-kernel.test"
const OLD_ENDPOINT = "ws://old-kernel.test"
const REPLACEMENT_ENDPOINT = "ws://replacement-kernel.test"

for (const [cliWorktree, kernelWorktree] of [["/repo-feature", "/repo"], ["/repo", "/repo-feature"]] as const) {
  reimageTest(`detached CLI worktree ${cliWorktree} survives kernel launch in ${kernelWorktree} through CreateSession`, async router => {
    const harness = createHarness(router, { interactivePlacement: true,
      initialTargets: { workspace: "/repo", worktree: cliWorktree },
      localLaunchTarget: { workspace: "/repo", worktree: kernelWorktree },
      localWorktrees: [
        { path: "/repo", branch: "main", current: kernelWorktree === "/repo" },
        { path: "/repo-feature", branch: "feature", current: kernelWorktree === "/repo-feature" },
      ],
    })
    try {
      assert.equal(harness.state().worktreeSelectionId, "")
      await harness.initialize()
      await new Promise(resolve => setImmediate(resolve))
      await harness.composition.startSessionFromWaitingRoomDefaults()
      assert.equal(requestCount(harness.local, "CreateSession"), 1)
      const request = harness.local.requests.find(request => requestKind(request) === "CreateSession")
      assert.equal(requestPayload(request, "CreateSession").workspace_id, "/repo")
      assert.equal(requestPayload(request, "CreateSession").worktree_id, cliWorktree)
      assert.equal(harness.state().worktreeSelectionId, `existing:${cliWorktree}`)
      assert.equal(requestCount(harness.local, "CreateWorkspaceWorktree"), 0)
    } finally { harness.cleanup() }
  })
}

function reimageTest(name: string, run: (router: TestRouter) => Promise<void>) {
  test(`production Waiting Room reimage composition: ${name}`, async () => {
  const router = installLocalIpcClientTestRouter()
  try {
    await run(router)
  } finally {
    router.restore()
  }
  })
}

reimageTest("observes the old kernel transactionally, rolls back failure, and requires confirmation", async (router) => {
      const harness = createHarness(router, { observationFailure: new Error("old observation failed") })
      try {
        await harness.initialize()
        await assert.rejects(
          harness.composition.reimageManagedEnvironment("environment-1", "prepare"),
          /old observation failed/,
        )

        assert.equal(harness.observedThroughEndpoint, OLD_ENDPOINT)
        assert.equal(harness.client.currentClient(), harness.local.client)
        assert.equal(harness.old.closeCount, 1)
        assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 0)
        await assert.rejects(
          harness.composition.reimageManagedEnvironment("environment-1", "confirm"),
          /Prepare this managed reimage/,
        )
      } finally {
        harness.cleanup()
      }
    })

reimageTest("keeps destructive admission local and launches the exact replacement identity", async (router) => {
      const harness = createHarness(router)
      try {
        await harness.initialize()
        const prepared = await harness.composition.reimageManagedEnvironment("environment-1", "prepare")

        assert.match(prepared.message, /Destructive confirmation required/)
        assert.match(prepared.message, /reimage environment-1 confirm/)
        assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(harness.client.currentClient(), harness.local.client)

        const completed = await harness.composition.reimageManagedEnvironment("environment-1", "confirm")

        assert.match(completed.message, /pivoted to the fresh kernel, and passed Project setup/)
        assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(harness.cloudPaths.filter(path => path.endsWith("/reimage")).length, 1)
        assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(requestCount(harness.replacement, "RequestManagedEnvironmentReimage"), 0)
        assert.deepEqual(resolveTargets(harness.local), [
          { kernelRef: "kernel-old", machineRef: "machine-old" },
          { kernelRef: "kernel-new", machineRef: "machine-new" },
        ])
        assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
        assert.equal(requestCount(harness.local, "CreateSession"), 0)
        assert.equal(requestCount(harness.replacement, "GetManagedContextLaunchTarget"), 0)
        assert.deepEqual(harness.attachments, [{ sessionId: "session-new", created: true }])
        assert.equal(harness.state().selectedMachineRef, managedEnvironmentMachineRef("environment-1"))
        const selectedReplacement = harness.composition.waitingRoomTargets()
          .managedEnvironmentCatalog?.environments[0]
        assert.equal(selectedReplacement?.runtimeMachineId, "machine-new")
        assert.equal(selectedReplacement?.runtimeKernelId, "kernel-new")
        assert.equal(harness.client.currentClient().socketPath, REPLACEMENT_ENDPOINT)
      } finally {
        harness.cleanup()
      }
    })

reimageTest("does not report cutover success when Project setup fails", async (router) => {
      const harness = createHarness(router, {
        contextPlan: sourcePlan(),
        setupFailure: new Error("dependency validation failed"),
      })
      try {
        await harness.initialize()
        await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
        let result: Awaited<ReturnType<typeof harness.composition.reimageManagedEnvironment>> | undefined

        await assert.rejects(async () => {
          result = await harness.composition.reimageManagedEnvironment("environment-1", "confirm")
        }, /dependency validation failed/)

        assert.equal(result, undefined)
        assert.equal(requestCount(harness.replacement, "StartProjectEnvironmentSetup"), 1)
        assert.equal(requestCount(harness.replacement, "DeleteSession"), 1)
        assert.equal(harness.attachments.length, 0)
        assert.equal(harness.client.currentClient(), harness.local.client)
      } finally {
        harness.cleanup()
      }
    })

reimageTest("does not report cutover success when replacement attachment fails", async (router) => {
      const harness = createHarness(router, { attachFailure: new Error("attach rejected") })
      try {
        await harness.initialize()
        await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
        let result: Awaited<ReturnType<typeof harness.composition.reimageManagedEnvironment>> | undefined

        await assert.rejects(async () => {
          result = await harness.composition.reimageManagedEnvironment("environment-1", "confirm")
        }, /attach rejected/)

        assert.equal(result, undefined)
        assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
        assert.equal(requestCount(harness.replacement, "DeleteSession"), 1)
        assert.deepEqual(harness.rollbacks, ["session-new"])
        assert.equal(harness.client.currentClient(), harness.local.client)
      } finally {
        harness.cleanup()
      }
    })
reimageTest("MP-02/MP-08/MP-11 stopped enrolled launch retains the selected second kernel through lifecycle callbacks", async (router) => {
  const harness = createHarness(router, { stoppedEnrolled: true })
  try {
    await harness.initialize()
    harness.selectKernel("kernel-selected")
    await harness.composition.startSessionFromWaitingRoomDefaults({ kind: "existing", environmentId: "environment-1" })
    assert.deepEqual(resolveTargets(harness.local), [{ kernelRef: "kernel-selected", machineRef: "machine-old" }])
    assert.equal(harness.state().selectedKernelRef, "kernel-selected")
    assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
    assert.equal(requestCount(harness.local, "CreateSession"), 0)
    assert.equal(requestCount(harness.local, "RequestManagedEnvironmentLifecycle"), 0)
    assert.equal(harness.cloudPaths.filter(path => path.endsWith("/lifecycle")).length, 1)
    assert.equal(requestCount(harness.replacement, "GetManagedContextLaunchTarget"), 0)
    assert.deepEqual(harness.attachments, [{ sessionId: "session-new", created: true }])
  } finally { harness.cleanup() }
})

reimageTest("selecting a ready managed machine browses its workspace without replacing home control authority", async router => {
  let finishWorktrees!: (response: unknown) => void
  const worktrees = new Promise(resolve => { finishWorktrees = resolve })
  const harness = createHarness(router, { interactivePlacement: true, managedWorktrees: worktrees })
  try {
    await harness.initialize()
    harness.composition.reconcileWaitingRoom({ ...harness.state(),
      selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old",
    })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.client.currentClient(), harness.local.client)
    assert.equal(harness.local.closeCount, 0)
    assert.equal(harness.old.closeCount, 0)
    assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
    assert.equal(harness.state().worktreeSelectionId, "")
    assert.equal(requestCount(harness.old, "ListWorkspaceWorktrees"), 1)
    assert.equal(requestCount(harness.local, "ListWorkspaceWorktrees"), 1)
    // Control remains available while a separate workspace read is in flight.
    const prepared = await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
    assert.match(prepared.message, /Destructive confirmation required/)
    finishWorktrees({ WorkspaceWorktreesListed: { worktrees: [{ path: "/home/managed", branch: "main", current: true }] } })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.old.closeCount, 2)
    assert.equal(harness.state().worktreeSelectionId, "existing:/home/managed")
    await harness.composition.refreshWaitingRoomDataNow()
    assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
    assert.equal(harness.composition.waitingRoomTargets().workspaceId, "/home/managed")
    assert.equal(harness.projects()[0]?.workspace_id, "/home/managed")
    harness.composition.applyWaitingRoomRowsChanged({
      inventoryVersion: "home-new", structuralVersion: "home-new", activityRevision: "home-new", sessions: [], removedSessionIds: [],
      projects: [{ id: "home-project", owner_user_id: "user-1", workspace_id: "/home/local", name: "Home", kind: "named", status: "active", created_at_ms: 1, updated_at_ms: 2,
        session_count: 0, joined_collaborator_count: 0, pending_collaboration_invite_count: 0 }],
    })
    assert.equal(harness.projects()[0]?.workspace_id, "/home/managed")
    assert.equal(requestCount(harness.local, "GetManagedEnvironmentReimagePreflight"), 0)
    assert.equal(harness.cloudPaths.filter(path => path.endsWith("/reimage/preflight")).length, 1)
    assert.equal(requestCount(harness.old, "GetManagedEnvironmentReimagePreflight"), 0)
    assert.equal(harness.observedThroughEndpoint, OLD_ENDPOINT)
    assert.equal(harness.client.currentClient(), harness.local.client)
    await harness.composition.reimageManagedEnvironment("environment-1", "confirm")
    assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 0)
    assert.equal(harness.cloudPaths.filter(path => path.endsWith("/reimage")).length, 1)
    assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
    assert.equal(requestCount(harness.replacement, "RequestManagedEnvironmentReimage"), 0)
    assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
  } finally { finishWorktrees({ WorkspaceWorktreesListed: { worktrees: [] } }); harness.cleanup() }
})

reimageTest("remote to local return restores local inventory scope and managed reimage admission", async router => {
  const harness = createHarness(router, { interactivePlacement: true })
  try {
    await harness.initialize()
    harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: "machine-old", selectedKernelRef: "kernel-old" })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.client.currentClient().socketPath, OLD_ENDPOINT)
    harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: "local", selectedKernelRef: "local" })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.client.currentClient().socketPath, LOCAL_ENDPOINT)
    assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/local")
    assert.deepEqual(harness.sessions().map(session => session.id).sort(), ["kernel-local-session", "kernel-old-session"])
    // Inspect admission before another placement callback can alter authority.
    harness.selectManagedWithoutReconcile()
    const prepared = await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
    assert.match(prepared.message, /Destructive confirmation required/)
    assert.equal(requestCount(harness.local, "GetManagedEnvironmentReimagePreflight"), 0)
    assert.equal(harness.cloudPaths.filter(path => path.endsWith("/reimage/preflight")).length, 1)
    assert.equal(requestCount(harness.old, "GetManagedEnvironmentReimagePreflight"), 0)
    assert.equal(harness.client.currentClient().socketPath, LOCAL_ENDPOINT)
  } finally { harness.cleanup() }
})

reimageTest("ready managed to New machine restores home source before Current Project creation and session launch", async router => {
  const contextPlan: ManagedEnvironmentContextPlan = { ...sourcePlan(), kernelContext: "empty",
    source: { ...sourcePlan().source!, sourceTargetId: "kernel-local", machineId: "machine-local", kernelId: "kernel-local" },
    developmentSetup: { kind: "source_project", projectId: "kernel-local-project",
      repositories: [{ role: "primary", workspaceId: "/home/local", worktreeId: "/home/local" }] },
  }
  const harness = createHarness(router, { interactivePlacement: true, contextPlan,
    contextSources: [{ sourceTargetId: "kernel-local", machineId: "machine-local", kernelId: "kernel-local", label: "Home" }],
  })
  try {
    await harness.initialize()
    harness.composition.reconcileWaitingRoom({ ...harness.state(),
      selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old",
    })
    await new Promise(resolve => setImmediate(resolve))
    harness.composition.reconcileWaitingRoom({ ...harness.state(), projectSelectionId: "existing:kernel-old-project" })
    assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
    assert.equal(harness.projects()[0]?.id, "kernel-old-project")
    assert.equal(harness.state().projectSelectionId, "existing:kernel-old-project")

    harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: NEW_MANAGED_MACHINE_REF })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.state().selectedKernelRef, "")
    assert.equal(harness.client.currentClient(), harness.local.client)
    assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/local")
    assert.equal(harness.composition.waitingRoomTargets().worktreePath, "/home/local")
    assert.equal(harness.composition.waitingRoomTargets().workspaceId, "/home/local")
    assert.equal(harness.projects()[0]?.id, "kernel-local-project")
    assert.equal(harness.state().projectSelectionId, "default")
    assert.equal(harness.state().worktreeSelectionId, "existing:/home/local")

    harness.composition.reconcileWaitingRoom({ ...harness.state(), projectSelectionId: "existing:kernel-local-project",
      managedDevelopmentMode: "current_project", managedKernelContext: "empty",
    })
    await harness.composition.startSessionFromWaitingRoomDefaults()
    assert.equal(requestCount(harness.local, "CreateManagedEnvironment"), 0)
    const request = harness.cloudRequests.find(request => request.path === "/managed-environments" && request.body)
    assert.deepEqual(request?.body.contextPlan, {
      sourceTargetId: "kernel-local", kernelContext: "empty", developmentSetup: contextPlan.developmentSetup,
      providerAccounts: { kind: "none" }, gitCredentials: { kind: "none" },
    })
    assert.equal(requestCount(harness.old, "CreateManagedEnvironment"), 0)
    assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
    const sessionRequest = harness.replacement.requests.find(request => requestKind(request) === "CreateSession")
    assert.deepEqual(requestPayload(sessionRequest, "CreateSession").project_selection, {
      kind: "existing", project_id: "kernel-local-project",
    })
    assert.deepEqual(harness.attachments, [{ sessionId: "session-new", created: true }])
  } finally { harness.cleanup() }
})

for (const machineRef of ["local", NEW_MANAGED_MACHINE_REF]) {
  reimageTest(`returning to ${machineRef} while a managed workspace read is pending discards the late worktrees and closes the read client`, async router => {
    let finishWorktrees!: (response: unknown) => void
    const harness = createHarness(router, { interactivePlacement: true,
      managedWorktrees: new Promise(resolve => { finishWorktrees = resolve }),
    })
    try {
      await harness.initialize()
      harness.composition.reconcileWaitingRoom({ ...harness.state(),
        selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old",
      })
      await new Promise(resolve => setImmediate(resolve))
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
      harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: machineRef, selectedKernelRef: "local" })
      await new Promise(resolve => setImmediate(resolve))
      finishWorktrees({ WorkspaceWorktreesListed: { worktrees: [{ path: "/home/managed", branch: "main", current: true }] } })
      await new Promise(resolve => setImmediate(resolve))
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/local")
      assert.equal(harness.state().worktreeSelectionId, "existing:/home/local")
      assert.equal(harness.projects()[0]?.id, "kernel-local-project")
      assert.equal(harness.composition.waitingRoomTargets().workspaceId, "/home/local")
      assert.equal(harness.client.currentClient(), harness.local.client)
      assert.equal(harness.local.closeCount, 0)
      assert.equal(harness.old.closeCount, 1)
    } finally { finishWorktrees({ WorkspaceWorktreesListed: { worktrees: [] } }); harness.cleanup() }
  })
}

reimageTest("returning to the original home does not grant local authority to a directly targeted CLI", async router => {
  const harness = createHarness(router, { interactivePlacement: true, initialTargetKernelId: "kernel-local" })
  try {
    await harness.initialize()
    harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: "machine-old", selectedKernelRef: "kernel-old" })
    await new Promise(resolve => setImmediate(resolve))
    harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: "local", selectedKernelRef: "local" })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.client.currentClient().socketPath, LOCAL_ENDPOINT)
    harness.selectManagedWithoutReconcile()
    await assert.rejects(harness.composition.reimageManagedEnvironment("environment-1", "prepare"), /Return to the local kernel/)
    assert.equal(requestCount(harness.local, "GetManagedEnvironmentReimagePreflight"), 0)
  } finally { harness.cleanup() }
})

reimageTest("MP-08 / MP-11 signed-in catalog and creation use human HTTP with kernel-owned launch", async router => {
  const harness = createHarness(router)
  try {
    await harness.initialize()
    assert.equal(harness.composition.waitingRoomTargets().managedEnvironmentCatalog?.computeClasses[0]?.computeClass, "agent-small")
    await harness.composition.startSessionFromWaitingRoomDefaults({kind: "new", region: "hel1", computeClass: "agent-small", managedRepositoryRoot: "/home/chariox", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan: {sourceTargetId: null, kernelContext: "empty", developmentSetup: {kind: "empty"}, providerAccounts: {kind: "none"}, gitCredentials: {kind: "none"}}})
    assert.ok(harness.cloudPaths.includes("/managed-environments"))
    assert.equal(requestCount(harness.local, "CreateManagedEnvironment"), 0)
    assert.equal(requestCount(harness.replacement, "GetManagedContextLaunchTarget"), 1)
    assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
  } finally {harness.cleanup()}
})

// MP-08 / MP-11: exercise the production callbacks with enrollment-only status
// and terminal-private human authority, including rejection before STOP.
for (const action of ["create", "reimage"] as const) for (const denied of [false, true]) reimageTest(`MP-08 / MP-11 selected-provider ${action} ${denied ? "fails before Cloud mutation" : "preflights on the source kernel"}`, async router => {
  const plan = {...emptyPlan(), providerAccounts: {kind: "selected" as const, accounts: [{provider: "codex", accountProfile: "fixture"}]}}
  const harness = createHarness(router, {contextPlan: plan, ...(denied ? {portabilityFailure: new Error("no transferable credentials")} : {})})
  try {
    await harness.initialize()
    if (action === "reimage") await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
    const before = harness.cloudPaths.length
    const request = action === "reimage"
      ? harness.composition.reimageManagedEnvironment("environment-1", "confirm")
      : harness.composition.startSessionFromWaitingRoomDefaults({kind: "new", region: "hel1", computeClass: "agent-small", managedRepositoryRoot: "/home/chariox", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan: {sourceTargetId: null, kernelContext: plan.kernelContext, developmentSetup: plan.developmentSetup, providerAccounts: plan.providerAccounts, gitCredentials: plan.gitCredentials}})
    if (denied) await assert.rejects(request, /no transferable credentials/)
    else await request
    assert.equal(requestCount(harness.local, "PreflightProviderAccountPortability"), 1)
    const preflightRequest = harness.local.requests.find(request => requestKind(request) === "PreflightProviderAccountPortability")
    assert.deepEqual(preflightRequest, {PreflightProviderAccountPortability: {providerAccounts: plan.providerAccounts}})
    assert.equal(requestCount(harness.old, "PreflightProviderAccountPortability"), 0)
    assert.equal(requestCount(harness.replacement, "PreflightProviderAccountPortability"), 0)
    if (denied) assert.equal(harness.cloudPaths.length, before, "no STOP/provisioning after failed preflight")
    else assert.ok(harness.cloudPaths.slice(before).some(path => action === "reimage" ? path.endsWith("/reimage") : path === "/managed-environments"))
  } finally {harness.cleanup()}
})

type TestEndpoint = {
  readonly client: LocalIpcClient
  readonly requests: unknown[]
  closeCount: number
  send(request: unknown): Promise<unknown>
}

reimageTest("defaults to current worktree after a delayed selected-machine inventory", async router => {
  let finishWorktrees!: (response: unknown) => void
  const worktrees = new Promise(resolve => { finishWorktrees = resolve })
  const harness = createHarness(router, { interactivePlacement: true, managedWorktrees: worktrees })
  try {
    await harness.initialize()
    harness.composition.reconcileWaitingRoom({ ...harness.state(),
      selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old" })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(requestCount(harness.old, "ListWorkspaceWorktrees"), 1)
    assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
    assert.equal(harness.state().worktreeSelectionId, "")
    finishWorktrees({ WorkspaceWorktreesListed: { worktrees: [
      { path: "/home/other", branch: "other", current: false },
      { path: "/home/managed", branch: "main", current: true },
    ] } })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(harness.state().worktreeSelectionId, "existing:/home/managed")
    assert.equal(stageWaitingRoomWorktreeSelection(harness.state().worktreeSelectionId).ok, true)
    assert.equal(await resolvePendingWaitingRoomWorktreePath("/home/managed", "/home/managed", {
      createWorktree: async () => { throw new Error("launch must not create a worktree by default") },
    }), "/home/managed")
  } finally { finishWorktrees({ WorkspaceWorktreesListed: { worktrees: [] } }); harness.cleanup() }
})

for (const disabledState of ["not-repository", "unborn"] as const) {
  reimageTest(`managed workspace refresh rechecks ${disabledState} after git repair without losing home authority`, async router => {
    let repositoryState: "not-repository" | "unborn" | "ready" = disabledState
    const harness = createHarness(router, { interactivePlacement: true, managedRepositoryState: () => repositoryState })
    try {
      await harness.initialize()
      harness.composition.reconcileWaitingRoom({ ...harness.state(),
        selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old" })
      await new Promise(resolve => setImmediate(resolve))
      assert.ok(waitingRoomWorktreeDisabledHint())
      assert.deepEqual(waitingRoomWorktreeOptions(), [])
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
      // Git repair does not change the waiting-room snapshot or its workspace.
      repositoryState = "ready"
      await harness.composition.refreshWaitingRoomDataNow()
      assert.equal(waitingRoomWorktreeDisabledHint(), null)
      assert.ok(waitingRoomWorktreeOptions().some(option => option.id === "create-worktree"))
      assert.equal(harness.state().worktreeSelectionId, "existing:/home/managed")
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/home/managed")
      assert.equal(harness.client.currentClient(), harness.local.client)
      assert.equal(harness.local.closeCount, 0)
      assert.equal(harness.old.closeCount, 2)
      assert.equal(requestCount(harness.old, "ListWorkspaceWorktrees"), 2)
      assert.equal(requestCount(harness.old, "GetWorkspaceGitOverview"), 1)
      await harness.composition.refreshWaitingRoomData()
      assert.equal(requestCount(harness.old, "ListWorkspaceWorktrees"), 2)
    } finally { harness.cleanup() }
  })
}

for (const browsePending of [false, true]) {
  reimageTest(`Workspace command reads the selected managed kernel with inventory browse pending=${browsePending}`, async router => {
    let finishInventory!: (response: unknown) => void
    const inventory = new Promise(resolve => { finishInventory = resolve })
    const harness = createHarness(router, { interactivePlacement: true,
      ...(browsePending ? { managedInventory: inventory } : {}),
      managedFilesystemRequest: request => {
        assert.equal(requestKind(request), "ListWorkspaceWorktrees")
        const path = requestPayload(request, "ListWorkspaceWorktrees").workspace_id
        return { WorkspaceWorktreesListed: { worktrees: [{ path, branch: "remote-only", current: true }] } }
      },
    })
    try {
      await harness.initialize()
      const homeWorkspace = harness.composition.waitingRoomTargets().workspacePath
      harness.composition.reconcileWaitingRoom({ ...harness.state(),
        selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old" })
      await new Promise(resolve => setImmediate(resolve))
      await harness.workspaceCommand("/remote-only/repo")
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/remote-only/repo")
      assert.equal(harness.state().worktreeSelectionId, "existing:/remote-only/repo")
      assert.equal(waitingRoomWorktreeDisabledHint(), null)
      assert.equal(requestCount(harness.local, "ListWorkspaceWorktrees"), 1)
      assert.equal(requestCount(harness.local, "GetWorkspaceGitOverview"), 0)
      assert.ok(harness.old.requests.some(request => requestKind(request) === "ListWorkspaceWorktrees"
        && requestPayload(request, "ListWorkspaceWorktrees").workspace_id === "/remote-only/repo"))
      finishInventory(placementSnapshot("kernel-old", "machine-old", "/home/managed"))
      await new Promise(resolve => setImmediate(resolve))
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/remote-only/repo")
      assert.equal(harness.state().worktreeSelectionId, "existing:/remote-only/repo")
      assert.equal(harness.client.currentClient(), harness.local.client)
      assert.equal(harness.local.closeCount, 0)
      assert.equal(harness.old.closeCount, 2)
      harness.composition.reconcileWaitingRoom({ ...harness.state(), selectedMachineRef: "local", selectedKernelRef: "local" })
      await new Promise(resolve => setImmediate(resolve))
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, homeWorkspace)
      harness.composition.reconcileWaitingRoom({ ...harness.state(),
        selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old" })
      await new Promise(resolve => setImmediate(resolve))
      assert.equal(harness.composition.waitingRoomTargets().workspacePath, "/remote-only/repo")
    } finally { finishInventory(placementSnapshot("kernel-old", "machine-old", "/home/managed")); harness.cleanup() }
  })
}

type TestRouter = ReturnType<typeof installLocalIpcClientTestRouter>

function installLocalIpcClientTestRouter() {
  const endpoints = new Map<string, TestEndpoint>()
  const send = LocalIpcClient.prototype.send
  const close = LocalIpcClient.prototype.close
  LocalIpcClient.prototype.send = function <TResponse>(request: unknown): Promise<TResponse> {
    const endpoint = endpoints.get(this.socketPath)
    if (!endpoint) {
      return Promise.reject(new Error(`unexpected LocalIpcClient endpoint ${this.socketPath}`))
    }
    endpoint.requests.push(structuredClone(request))
    return endpoint.send(request) as Promise<TResponse>
  }
  LocalIpcClient.prototype.close = async function (): Promise<void> {
    const endpoint = endpoints.get(this.socketPath)
    if (!endpoint) {
      throw new Error(`unexpected LocalIpcClient close for ${this.socketPath}`)
    }
    endpoint.closeCount += 1
  }
  return {
    endpoint(socketPath: string, responder: (request: unknown) => Promise<unknown> | unknown): TestEndpoint {
      const endpoint: TestEndpoint = {
        client: new LocalIpcClient(socketPath),
        requests: [],
        closeCount: 0,
        send: async (request) => await responder(request),
      }
      endpoints.set(socketPath, endpoint)
      return endpoint
    },
    restore() {
      LocalIpcClient.prototype.send = send
      LocalIpcClient.prototype.close = close
      endpoints.clear()
    },
  }
}

function createHarness(router: TestRouter, options: {
  observationFailure?: Error
  contextPlan?: ManagedEnvironmentContextPlan
  setupFailure?: Error
  attachFailure?: Error
  portabilityFailure?: Error
  stoppedEnrolled?: boolean
  interactivePlacement?: boolean
  managedWorktrees?: Promise<unknown>
  managedInventory?: Promise<unknown>
  managedFilesystemRequest?: (request: unknown) => unknown
  managedRepositoryState?: () => "not-repository" | "unborn" | "ready"
  initialTargetKernelId?: string
  initialTargets?: { workspace: string; worktree: string }
  localLaunchTarget?: { workspace: string; worktree: string }
  localWorktrees?: Array<{ path: string; branch: string; current: boolean }>
  contextSources?: Array<{ sourceTargetId: string; machineId: string; kernelId: string; label: string }>
} = {}) {
  const contextPlan = options.contextPlan ?? emptyPlan()
  const oldEnvironment = environment({
    desiredRevision: 7,
    observedRevision: 7,
    runtimeMachineId: "machine-old",
    runtimeKernelId: "kernel-old",
    runtimeReleaseDigest: "sha256:old-release",
    contextPlan,
    ...(options.stoppedEnrolled ? { desiredState: "stopped", observedState: "stopped" } : {}),
  })
  const replacementEnvironment = environment({ contextPlan,
    ...(options.stoppedEnrolled ? { runtimeMachineId: "machine-old", runtimeKernelId: "kernel-old" } : {}) })
  const catalog = {
    computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
    contextSources: options.contextSources ?? [],
    environments: [oldEnvironment],
  }
  let mutableClient!: MutableLocalIpcClient
  let observedThroughEndpoint: string | null = null
  let environmentReads = 0
  const local = router.endpoint(LOCAL_ENDPOINT, async (request) => {
    if (/^(ListManagedEnvironmentCatalog|GetManagedEnvironment|CreateManagedEnvironment|RequestManagedEnvironment|PrepareManagedEnvironment)/.test(requestKind(request))) throw new Error("Cloud session is unavailable: kernel has enrollment authority only")
    if (requestKind(request) === "CloudRelayStatus") return {CloudRelayStatus: {profile: {api_url: "http://127.0.0.1:44123", account_id: "account-1", user_id: "user-1", kernel_id: "kernel-local", kernel_enrolled: true}}}
    switch (requestKind(request)) {
      case "PreflightProviderAccountPortability":
        if (options.portabilityFailure) throw options.portabilityFailure
        return {ProviderAccountPortabilityPreflightPassed: {}}
      case "GetWaitingRoomPublicSnapshot":
        if (options.localLaunchTarget) {
          const response = snapshotResponse("kernel-local", "machine-local", contextPlan)
          return { WaitingRoomPublicSnapshot: { snapshot: {
            ...response.WaitingRoomPublicSnapshot.snapshot,
            launch_target: { workspace_id: options.localLaunchTarget.workspace, worktree_id: options.localLaunchTarget.worktree },
          } } }
        }
        return options.interactivePlacement ? placementSnapshot("kernel-local", "machine-local", "/home/local")
          : snapshotResponse("kernel-local", "machine-local", contextPlan)
      case "ListWorkspaceWorktrees":
        return { WorkspaceWorktreesListed: { worktrees: options.localWorktrees ?? [{ path: "/home/local", branch: "main", current: true }] } }
      case "CreateSession": {
        const payload = requestPayload(request, "CreateSession")
        return { SessionCreated: { session: { ...runtimeSession(contextPlan), id: "session-local",
          workspace_id: payload.workspace_id, worktree_id: payload.worktree_id } } }
      }
      case "StartProjectEnvironmentSetup": {
        const input = requestPayload(request, "StartProjectEnvironmentSetup") as unknown as ProjectEnvironmentSetupStartInput
        return { ProjectEnvironmentSetupStarted: { status: setupStatus(input) } }
      }
      case "GetProviderCatalog":
        return { ProviderCatalog: { catalog: fallbackProviderCatalog() } }
      case "ListSlices":
        return { SlicesListed: { slices: [] } }
      case "ListManagedEnvironmentCatalog":
        return { ManagedEnvironmentCatalog: { catalog } }
      case "GetManagedEnvironmentReimagePreflight":
        return { ManagedEnvironmentReimagePreflight: { preflight: preflight() } }
      case "ResolveKernelClientConnection": {
        const payload = requestPayload(request, "ResolveKernelClientConnection")
        const old = payload.kernel_ref === "kernel-old"
        return {
          KernelClientConnectionResolved: {
            connection: {
              relay_url: old ? OLD_ENDPOINT : REPLACEMENT_ENDPOINT,
              relay_token: old ? "old-token" : "replacement-token",
              target_daemon_id: old ? "kernel-old" : options.stoppedEnrolled ? "kernel-selected" : "kernel-new",
              target_daemon_alias: old ? "old" : "replacement",
              machine_id: old || options.stoppedEnrolled ? "machine-old" : "machine-new",
              kernel_id: old ? "kernel-old" : options.stoppedEnrolled ? "kernel-selected" : "kernel-new",
            },
          },
        }
      }
      case "RequestManagedEnvironmentLifecycle":
        return { ManagedEnvironmentLifecycleRequested: { result: { ...reimageResult(replacementEnvironment, "start-key"), operation: { ...reimageResult(replacementEnvironment, "start-key").operation, kind: "start" } } } }
      case "CreateManagedEnvironment":
        return { ManagedEnvironmentCreated: { result: { ...reimageResult(replacementEnvironment, "create-key"),
          operation: { ...reimageResult(replacementEnvironment, "create-key").operation, kind: "create" } } } }
      case "RequestManagedEnvironmentReimage": {
        const payload = requestPayload(request, "RequestManagedEnvironmentReimage")
        return { ManagedEnvironmentReimageRequested: { result: reimageResult(replacementEnvironment, payload.idempotencyKey as string) } }
      }
      case "GetManagedEnvironment":
        return { ManagedEnvironment: { environment: options.stoppedEnrolled && environmentReads++ === 0 ? oldEnvironment : replacementEnvironment } }
      default:
        throw new Error(`unexpected local request ${requestKind(request)}`)
    }
  })
  const old = router.endpoint(OLD_ENDPOINT, async (request) => {
    if (["ListWorkspaceWorktrees", "GetWorkspaceGitOverview"].includes(requestKind(request)) && options.managedFilesystemRequest) {
      return options.managedFilesystemRequest(request)
    }
    if (requestKind(request) === "CloudRelayStatus") return {CloudRelayStatus: {profile: {api_url: "http://127.0.0.1:44123", account_id: "account-1", user_id: "user-1", kernel_enrolled: true}}}
    switch (requestKind(request)) {
      case "GetWaitingRoomPublicSnapshot":
        if (options.managedInventory) return options.managedInventory
        return options.interactivePlacement ? placementSnapshot("kernel-old", "machine-old", "/home/managed")
          : snapshotResponse("kernel-old", "machine-old", contextPlan)
      case "ListWorkspaceWorktrees":
        return options.managedWorktrees ?? { WorkspaceWorktreesListed: { worktrees: options.managedRepositoryState && options.managedRepositoryState() !== "ready"
          ? [] : [{ path: "/home/managed", branch: "main", current: true }] } }
      case "GetWorkspaceGitOverview":
        return { WorkspaceGitOverview: { overview: { repo_root: options.managedRepositoryState?.() === "not-repository" ? null : "/home/managed", compare_refs: [] } } }
      case "GetProviderCatalog":
        return { ProviderCatalog: { catalog: fallbackProviderCatalog() } }
      case "ResolveKernelClientConnection":
        assert.equal(requestPayload(request, "ResolveKernelClientConnection").kernel_ref, "kernel-local")
        return { KernelClientConnectionResolved: { connection: {
          relay_url: LOCAL_ENDPOINT, relay_token: "local-test-token", target_daemon_id: "kernel-local",
          kernel_id: "kernel-local", machine_id: "machine-local",
        } } }
      case "ListSlices":
        return { SlicesListed: { slices: [] } }
      case "ListManagedEnvironmentCatalog":
        return { ManagedEnvironmentCatalog: { catalog: { ...catalog, environments: [oldEnvironment] } } }
      case "ObserveManagedEnvironmentPreReimage":
        observedThroughEndpoint = mutableClient.currentClient().socketPath
        if (options.observationFailure) throw options.observationFailure
        return {
          ManagedEnvironmentPreReimageObserved: {
            acknowledgement: {
              environmentId: "environment-1",
              generation: 3,
              observedAt: "2026-09-22T00:00:00.000Z",
            },
          },
        }
      default:
        throw new Error(`unexpected old-kernel request ${requestKind(request)}`)
    }
  })
  const session = runtimeSession(contextPlan)
  const replacement = router.endpoint(REPLACEMENT_ENDPOINT, async (request) => {
    if (requestKind(request) === "CloudRelayStatus") return {CloudRelayStatus: {profile: {api_url: "http://127.0.0.1:44123", account_id: "account-1", user_id: "user-1", kernel_enrolled: true}}}
    switch (requestKind(request)) {
      case "GetWaitingRoomPublicSnapshot":
        return snapshotResponse(options.stoppedEnrolled ? "kernel-selected" : "kernel-new",
          options.stoppedEnrolled ? "machine-old" : "machine-new", contextPlan,
          options.stoppedEnrolled ? [{ kernel_id: "kernel-old", machine_id: "machine-old" },
            { kernel_id: "kernel-selected", machine_id: "machine-old" }] : [])
      case "ListSlices":
        return { SlicesListed: { slices: [] } }
      case "ListManagedEnvironmentCatalog":
        return { ManagedEnvironmentCatalog: { catalog: { ...catalog, environments: [replacementEnvironment] } } }
      case "GetManagedContextLaunchTarget":
        return { ManagedContextLaunchTarget: { target: launchTarget(contextPlan) } }
      case "CreateSession":
        return { SessionCreated: { session } }
      case "StartProjectEnvironmentSetup": {
        const input = requestPayload(request, "StartProjectEnvironmentSetup") as unknown as ProjectEnvironmentSetupStartInput
        return {
          ProjectEnvironmentSetupStarted: {
            status: setupStatus(input, options.setupFailure),
          },
        }
      }
      case "DeleteSession":
        return { SessionDeleted: { session } }
      default:
        throw new Error(`unexpected replacement-kernel request ${requestKind(request)}`)
    }
  })
  mutableClient = createMutableLocalIpcClient(local.client)

  const cacheDirectory = mkdtempSync(join(tmpdir(), "chariox-reimage-composition-"))
  const previousCacheDirectory = process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR
  process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR = cacheDirectory
  __setWaitingRoomWorktreeInventoryForTest(options.initialTargets ? null : {
    workspacePath: "/staged",
    currentWorktreePath: "/staged/worktree",
    options: [{
      id: "existing:/staged/worktree",
      kind: "existing",
      label: "staged worktree",
      path: "/staged/worktree",
      branch: "test",
      isCurrent: true,
    }],
  })
  let waitingRoomState: WaitingRoomState = {
    ...createWaitingRoomState([], fallbackProviderCatalog(), "opencode", "opencode/gpt-5.4", "high"),
    worktreeSelectionId: options.initialTargets ? "" : "existing:/staged/worktree",
    selectedMachineRef: options.interactivePlacement ? "local" : managedEnvironmentMachineRef("environment-1"),
    selectedKernelRef: options.interactivePlacement ? "local" : "kernel-old",
    projectSelectionId: contextPlan.developmentSetup.kind === "source_project"
      ? `existing:${contextPlan.developmentSetup.projectId}` : "default",
  }
  let ownershipRevision = 0
  let relayStatus = relayStatusFor("kernel-local", "machine-local")
  let inventoryStatus: "loading" | "ready" | "error" = "ready"
  let availableSessions: unknown[] = []
  let projects: unknown[] = []
  let remoteMachines: unknown[] = []
  let remoteKernels: unknown[] = []
  let providerAccounts: unknown[] = []
  let terminals: unknown[] = []
  let slices: unknown[] = []
  let externalSessions: unknown[] = []
  let externalPage = { hasMore: false, nextCursor: null as string | null }
  let pendingWorkspace = options.initialTargets?.workspace ?? ""
  let pendingWorktree = options.initialTargets?.worktree ?? ""
  let providerCatalog = fallbackProviderCatalog()
  let preferences = {}
  const attachments: Array<{ sessionId: string; created: boolean }> = []
  const rollbacks: string[] = []
  const setValue = <T>(current: T, update: T | ((value: T) => T)): T => (
    typeof update === "function" ? (update as (value: T) => T)(current) : update
  )
  const noop = () => {}
  // MP-08 / MP-11: terminal authority never enters the enrolled kernel.
  const cloudPaths: string[] = []
  const cloudRequests: Array<{path: string; body: any}> = []
  const previousFetch = globalThis.fetch
  const credential = {profile: {apiUrl: "http://127.0.0.1:44123", accountId: "account-1", userId: "user-1", clientId: "terminal"}, accessToken: "synthetic-human-access", publicKeyThumbprint: "a".repeat(64)}
  const cloudClient = new CloudClient({load: async () => credential, session: async () => credential} as any, () => ({publicKeyThumbprint: credential.publicKeyThumbprint}) as any)
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input)), body = init?.body ? JSON.parse(String(init.body)) : null
    cloudPaths.push(url.pathname)
    cloudRequests.push({path: url.pathname, body})
    assert.equal(new Headers(init?.headers).get("authorization"), "Bearer synthetic-human-access")
    if (body) {assert.equal(body.accountId, "account-1"); assert.equal(body.kernelCredential, undefined)}
    if (url.pathname.endsWith("/options")) return Response.json({computeClasses: catalog.computeClasses, contextSources: catalog.contextSources})
    if (url.pathname === "/managed-environments" && !body) return Response.json({environments: catalog.environments})
    if (url.pathname === "/managed-environments" && body) return Response.json({...reimageResult(replacementEnvironment, body.clientRequestId), operation: {...reimageResult(replacementEnvironment, body.clientRequestId).operation, kind: "create"}})
    if (url.pathname.endsWith("/reimage/preflight")) return Response.json(preflight())
    if (url.pathname.endsWith("/reimage/stop")) return Response.json({environment: {...oldEnvironment, runtimeGeneration: 3, desiredState: "stopped", observedState: "stopped", desiredRevision: 8, observedRevision: 8}, operation: {...reimageResult(replacementEnvironment, body.idempotencyKey).operation, kind: "stop", idempotencyKey: `reimage-stop:${createHash("sha256").update(body.idempotencyKey).digest("hex")}`}})
    if (url.pathname.endsWith("/reimage")) return Response.json(reimageResult(replacementEnvironment, body.idempotencyKey))
    if (url.pathname.endsWith("/lifecycle")) return Response.json({...reimageResult(replacementEnvironment, body.idempotencyKey), operation: {...reimageResult(replacementEnvironment, body.idempotencyKey).operation, kind: body.action}})
    if (url.pathname === "/managed-environments/environment-1") return Response.json({environment: options.stoppedEnrolled && environmentReads++ === 0 ? oldEnvironment : replacementEnvironment})
    throw new Error(`unexpected Cloud route ${url.pathname}`)
  }
  const deps: CliWaitingRoomCompositionDeps = {
    cloudClient,
    client: mutableClient,
    options: { clientId: "client-1", provider: "opencode", model: "opencode/gpt-5.4", effort: "high", targetDaemonId: options.initialTargetKernelId,
      ...(options.initialTargets ? { detached: true, ...options.initialTargets } : {}),
    },
    appLogger: { warn: noop, info: noop, debug: noop },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
    isAttached: () => false,
    kernelConnected: () => true,
    waitingRoomState: () => waitingRoomState,
    setWaitingRoomState: (update: WaitingRoomState | ((value: WaitingRoomState) => WaitingRoomState)) => {
      waitingRoomState = setValue(waitingRoomState, update)
      ownershipRevision += 1
    },
    setWaitingRoomStateProjection: (next: WaitingRoomState) => { waitingRoomState = next },
    waitingRoomLaunchOwnershipRevision: () => ownershipRevision,
    availableSessions: () => availableSessions,
    setAvailableSessions: (update: unknown) => { availableSessions = setValue(availableSessions, update as never) },
    waitingRoomProjects: () => projects,
    setWaitingRoomProjects: (update: unknown) => { projects = setValue(projects, update as never) },
    providerCatalogState: () => providerCatalog,
    setProviderCatalogState: (next: typeof providerCatalog) => { providerCatalog = next },
    providerCommandCatalogState: () => ({}),
    setProviderCommandCatalogState: noop,
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => inventoryStatus,
    setWaitingRoomInventoryStatus: (next: typeof inventoryStatus) => { inventoryStatus = next },
    waitingRoomHiddenKernelController: { hideKernel: noop, isKernelHidden: () => false },
    relayStatusState: () => relayStatus,
    setRelayStatusState: (next: typeof relayStatus) => { relayStatus = next },
    remoteMachinesState: () => remoteMachines,
    setRemoteMachinesState: (update: unknown) => { remoteMachines = setValue(remoteMachines, update as never) },
    remoteKernelsState: () => remoteKernels,
    setRemoteKernelsState: (update: unknown) => { remoteKernels = setValue(remoteKernels, update as never) },
    providerAccountsState: () => providerAccounts,
    setProviderAccountsState: (update: unknown) => { providerAccounts = setValue(providerAccounts, update as never) },
    terminalsState: () => terminals,
    setTerminalsState: (update: unknown) => { terminals = setValue(terminals, update as never) },
    slicesState: () => slices,
    setSlicesState: (update: unknown) => { slices = setValue(slices, update as never) },
    externalProviderSessionsState: () => externalSessions,
    setExternalProviderSessionsState: (update: unknown) => { externalSessions = setValue(externalSessions, update as never) },
    externalProviderSessionsPageState: () => externalPage,
    setExternalProviderSessionsPageState: (next: typeof externalPage) => { externalPage = next },
    pendingWorkspaceTarget: () => pendingWorkspace,
    setPendingWorkspaceTarget: (next: string) => { pendingWorkspace = next },
    pendingWorktreeTarget: () => pendingWorktree,
    setPendingWorktreeTarget: (next: string) => { pendingWorktree = next },
    preferencesState: () => preferences,
    setPreferencesState: (update: unknown) => { preferences = setValue(preferences, update as never) },
    setThemeRevision: noop,
    resetTranscriptSyntax: noop,
    applyResponseLayout: noop,
    renderCommandCenter: noop,
    rebuildTranscript: noop,
    updateSessionChrome: noop,
    syncCommandCenter: noop,
    handleCloudCommand: async () => {},
    setPromptText: noop,
    focusPrompt: noop,
    openTerminalPairingDialog: async () => {},
    openSessionBrowserDialog: noop,
    attachBinding: async (attachedSession: RuntimeSession, created: boolean) => {
      if (options.attachFailure) throw options.attachFailure
      attachments.push({ sessionId: attachedSession.id, created })
    },
    rollbackAttachedSession: async (sessionId: string) => { rollbacks.push(sessionId) },
    flashFooter: noop,
    setKernelConnected: noop,
    setDaemonDisconnected: noop,
    sessionBrowserOpen: () => false,
    closeSessionBrowserDialog: noop,
    focusedProviderRun: () => null,
    focusedAgent: () => null,
    focusedAgentId: () => null,
    providerRunState: () => null,
    sessionState: () => null,
    applySessionState: noop,
    setProviderRunState: noop,
    appendNotice: noop,
  }
  const composition = createCliWaitingRoomComposition(deps)

  return {
    composition,
    cloudPaths,
    cloudRequests,
    initialize: () => composition.refreshWaitingRoomDataNow(),
    workspaceCommand: async (path: string) => {
      const commands = createCliCommandActionComposition({ ...deps, ...composition,
        initialWorkspaceTarget: "/home/local", initialWorktreeTarget: "/home/local",
        pendingWorkspaceTarget: () => pendingWorkspace, pendingWorktreeTarget: () => pendingWorktree,
        setPendingWorkspaceTarget: deps.setPendingWorkspaceTarget, setPendingWorktreeTarget: deps.setPendingWorktreeTarget,
        isAttached: () => false, sessionState: () => null,
      } as unknown as Parameters<typeof createCliCommandActionComposition>[0])
      await commands.handleWorkspaceCommand({ kind: "workspace", raw: `/workspace ${path}`, args: [path] })
      await new Promise(resolve => setImmediate(resolve))
    },
    client: mutableClient,
    local,
    old,
    replacement,
    attachments,
    rollbacks,
    state: () => waitingRoomState,
    sessions: () => availableSessions as Array<{ id: string }>,
    projects: () => projects as Array<{ id: string; workspace_id: string }>,
    selectManagedWithoutReconcile: () => {
      waitingRoomState = { ...waitingRoomState, selectedMachineRef: managedEnvironmentMachineRef("environment-1"), selectedKernelRef: "kernel-old" }
      ownershipRevision += 1
    },
    selectKernel: (kernel: string) => { waitingRoomState = { ...waitingRoomState, selectedKernelRef: kernel }; ownershipRevision += 1 },
    get observedThroughEndpoint() { return observedThroughEndpoint },
    cleanup() {
      cloudClient.stop()
      globalThis.fetch = previousFetch
      __setWaitingRoomWorktreeInventoryForTest(null)
      if (previousCacheDirectory === undefined) {
        delete process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR
      } else {
        process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR = previousCacheDirectory
      }
      rmSync(cacheDirectory, { recursive: true, force: true })
    },
  }
}

function requestKind(request: unknown): string {
  assert.ok(request && typeof request === "object" && !Array.isArray(request))
  const keys = Object.keys(request)
  assert.equal(keys.length, 1)
  return keys[0]!
}

function requestPayload(request: unknown, kind: string): Record<string, unknown> {
  assert.equal(requestKind(request), kind)
  return (request as Record<string, Record<string, unknown>>)[kind]!
}

function requestCount(endpoint: TestEndpoint, kind: string): number {
  return endpoint.requests.filter((request) => requestKind(request) === kind).length
}

function resolveTargets(endpoint: TestEndpoint) {
  return endpoint.requests
    .filter((request) => requestKind(request) === "ResolveKernelClientConnection")
    .map((request) => {
      const payload = requestPayload(request, "ResolveKernelClientConnection")
      return { kernelRef: payload.kernel_ref, machineRef: payload.machine_ref }
    })
}

function snapshotResponse(kernelId: string, machineId: string, plan: ManagedEnvironmentContextPlan,
  kernels: Array<{ kernel_id: string; machine_id: string }> = []) {
  return {
    WaitingRoomPublicSnapshot: {
      snapshot: {
        schema_version: 11,
        inventory_version: `${kernelId}:inventory`,
        structural_version: `${kernelId}:structural`,
        activity_revision: `${kernelId}:activity`,
        generated_at_ms: 1,
        sessions: [],
        projects: plan.developmentSetup.kind === "source_project" ? [{
          id: plan.developmentSetup.projectId, name: "Selected Project",
          kind: "named", status: "active", workspace_id: "/staged",
          workspace_ids: ["/staged"], created_at_ms: 1, updated_at_ms: 1,
        }] : [],
        relay_status: relayStatusFor(kernelId, machineId),
        remote_machines: [],
        remote_kernels: kernels,
        terminals: [],
        provider_accounts: [],
        git_credentials: [],
      },
    },
  }
}

function placementSnapshot(kernelId: string, machineId: string, workspace: string) {
  const response = snapshotResponse(kernelId, machineId, emptyPlan())
  return { WaitingRoomPublicSnapshot: { snapshot: {
    ...response.WaitingRoomPublicSnapshot.snapshot,
    launch_target: { workspace_id: workspace, worktree_id: workspace },
    sessions: [{ id: `${kernelId}-session`, workspace_id: workspace, worktree_id: workspace, created_at_ms: 1 }],
    projects: [{ id: `${kernelId}-project`, owner_user_id: "user-1", workspace_id: workspace, name: "Workspace Project", kind: "named", status: "active", created_at_ms: 1, updated_at_ms: 1,
      session_count: 1, joined_collaborator_count: 0, pending_collaboration_invite_count: 0 }],
    remote_machines: [{ machine_id: "machine-old", machine_alias: "Managed old" }],
    remote_kernels: [{ kernel_id: "kernel-old", machine_id: "machine-old", kernel_alias: "Managed old" }],
  } } }
}

function relayStatusFor(kernelId: string, machineId: string) {
  return {
    configured: true,
    connected: true,
    relay_url: "wss://relay.test",
    daemon_id: kernelId,
    daemon_alias: kernelId,
    machine_id: machineId,
    machine_alias: machineId,
  }
}

function preflight(): ManagedEnvironmentReimagePreflight {
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
    },
    desiredRelease: {
      providerId: "hetzner",
      providerImageId: "image-2",
      providerProfileId: "path1",
      providerProfileDigest: "sha256:profile",
      runtimeReleaseDigest: "sha256:new-release",
      runtimeSourceCommit: "1".repeat(40),
      runtimeSourceTree: "2".repeat(40),
    },
  }
}

function emptyPlan(): ManagedEnvironmentContextPlan {
  return {
    schemaVersion: 1,
    contextId: "context-new",
    planDigest: "sha256:context",
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
      relayRealmId: "realm-source",
      machineId: "machine-source",
      kernelId: "kernel-source",
      keyThumbprint: "sha256:source-key",
    },
    kernelContext: "source_kernel",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [{ role: "primary", workspaceId: "workspace-primary", worktreeId: "worktree-primary" }],
    },
  }
}

function environment(overrides: Partial<ManagedEnvironmentSummary> = {}): ManagedEnvironmentSummary {
  return {
    environmentId: "environment-1",
    accountId: "account-1",
    createdByUserId: "user-1",
    name: "Managed agent",
    region: "hel1",
    computeClass: "agent-small",
    managedRepositoryRoot: "/home/chariox",
    desiredState: "running",
    observedState: "ready",
    desiredRevision: 8,
    observedRevision: 8,
    runtimeMachineId: "machine-new",
    runtimeKernelId: "kernel-new",
    runtimeReleaseDigest: "sha256:new-release",
    contextPlan: emptyPlan(),
    contextManifestDigest: "sha256:manifest",
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    lastErrorCode: null,
    lastErrorMessage: null,
    createdAt: "2026-09-22T00:00:00.000Z",
    updatedAt: "2026-09-22T00:01:00.000Z",
    ...overrides,
  }
}

function reimageResult(
  replacement: ManagedEnvironmentSummary,
  idempotencyKey: string,
): ManagedEnvironmentReimageResult {
  return {
    environment: replacement,
    operation: {
      operationId: "operation-1",
      environmentId: "environment-1",
      requestedByUserId: "user-1",
      kind: "reimage",
      idempotencyKey,
      requestDigest: `sha256:${"a".repeat(64)}`,
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
    },
  }
}

function launchTarget(plan: ManagedEnvironmentContextPlan) {
  return {
    environmentId: "environment-1",
    kernelId: "kernel-new",
    contextId: plan.contextId,
    planDigest: plan.planDigest,
    development: plan.developmentSetup.kind === "empty"
      ? { kind: "empty" as const, workspacePath: "/managed/empty/context-new" }
      : {
          kind: "from_source" as const,
          projectId: plan.developmentSetup.projectId,
          destinationRoot: "/managed/context-new",
          primaryRepositoryId: "repository-primary",
          repositories: [{
            repositoryId: "repository-primary",
            role: "primary" as const,
            targetDirectory: "primary",
            workspacePath: "/managed/context-new/primary",
            headSha: "a".repeat(40),
          }],
        },
  }
}

function runtimeSession(plan: ManagedEnvironmentContextPlan): RuntimeSession {
  const workspace = plan.developmentSetup.kind === "empty"
    ? "/managed/empty/context-new"
    : "/managed/context-new/primary"
  return {
    id: "session-new",
    project_id: plan.developmentSetup.kind === "empty" ? "default" : plan.developmentSetup.projectId,
    workspace_id: workspace,
    worktree_id: workspace,
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [{ id: "agent-1" } as RuntimeSession["agents"][number]],
    config_state: { version: 1, values: {}, updated_by_attachment_id: null },
  }
}

function setupStatus(
  input: ProjectEnvironmentSetupStartInput,
  failure?: Error,
): ProjectEnvironmentSetupStatus {
  return {
    operation_id: input.operationId,
    project_id: input.projectId,
    session_id: input.sessionId,
    agent_id: input.agentId,
    worker_id: input.targetWorkerId || "kernel-new",
    platform: input.targetPlatform || "linux",
    phase: failure ? "failed" : "ready",
    attempt: 1,
    progress_percent: failure ? 60 : 100,
    definition_digest: "sha256:definition",
    validation: {
      worker_id: input.targetWorkerId || "kernel-new",
      platform: input.targetPlatform || "linux",
      commands: [],
    },
    message: failure?.message ?? "ready",
    failure_code: failure ? "validation_failed" : null,
    failure_message: failure?.message ?? null,
    retryable: false,
    created_at_ms: 1,
    updated_at_ms: 2,
  }
}
