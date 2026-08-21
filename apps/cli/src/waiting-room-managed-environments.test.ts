import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { deriveWaitingRoomActivationDecision } from "./waiting-room-controller.js"
import { waitingRoomFocusTargets } from "./waiting-room-focus-targets.js"
import {
  cycleManagedCustomIdle,
  managedEnvironmentAutoStopPolicy,
  managedEnvironmentContextPlanInput,
  managedEnvironmentDraftBlockReason,
  managedEnvironmentMachineRef,
  NEW_MANAGED_MACHINE_REF,
} from "./waiting-room-managed-environments.js"
import { waitingRoomRows } from "./waiting-room-rows.js"
import { createWaitingRoomState, normalizeWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import { cycleWaitingRoomValue } from "./waiting-room-value-cycling.js"
import { __setWaitingRoomWorktreeInventoryForTest } from "./waiting-room-worktrees.js"

test("managed environments share the Machine selector without duplicating runtime Machines", () => {
  const normalized = normalizeWaitingRoomState({
    ...baseState(),
    selectedMachineRef: "local",
  }, [], catalog(), undefined, remote())
  const machineOptions = waitingRoomRows(normalized, [], catalog(), remote())
    .find((row) => row.id === "launch-machine")
  assert.equal(machineOptions?.value, "local")

  const managed = normalizeWaitingRoomState({
    ...normalized,
    selectedMachineRef: managedEnvironmentMachineRef("environment-1"),
    selectedKernelRef: "kernel-managed",
  }, [], catalog(), undefined, remote())
  assert.equal(
    waitingRoomRows(managed, [], catalog(), remote())
      .find((row) => row.id === "launch-machine")?.value,
    "Managed build · ready",
  )
  assert.equal(managed.selectedKernelRef, "kernel-managed")
})

test("new managed Machine selection reveals every conditional configuration row", () => {
  const state = normalizeWaitingRoomState({
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    selectedKernelRef: "",
  }, [], catalog(), undefined, remote())
  const rows = waitingRoomRows(state, [], catalog(), remote())
  assert.equal(rows.find((row) => row.id === "new")?.title, "Create machine and start session")
  assert.deepEqual(rows.filter((row) => row.id.startsWith("managed-")).map((row) => row.id), [
    "managed-compute",
    "managed-region",
    "managed-kernel-context",
    "managed-development",
    "managed-repositories",
    "managed-provider-accounts",
    "managed-git-credentials",
    "managed-auto-stop",
  ])
  assert.deepEqual(
    waitingRoomFocusTargets([], remote(), state)
      .filter((target) => target.focus.startsWith("managed-"))
      .map((target) => target.focus),
    rows.filter((row) => row.id.startsWith("managed-")).map((row) => row.id),
  )
})

test("clean kernel plus Current Project retains one source and Project repository defaults", () => {
  const state: WaitingRoomState = {
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    managedComputeClass: "agent-small",
    managedRegion: "hel1",
    managedKernelContext: "empty",
    managedContextSourceTargetId: "source-target-1",
    managedDevelopmentMode: "current_project",
    managedRepositoryMode: "project_defaults",
    projectSelectionId: "existing:project-1",
  }
  assert.equal(managedEnvironmentDraftBlockReason(state, remote()), null)
  assert.deepEqual(managedEnvironmentContextPlanInput(state, remote()), {
    sourceTargetId: "source-target-1",
    kernelContext: "empty",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [
        { role: "primary", workspaceId: "workspace-primary", worktreeId: "worktree-primary" },
        { role: "supporting", workspaceId: "workspace-supporting", worktreeId: null },
      ],
    },
    providerAccounts: { kind: "none" },
    gitCredentials: { kind: "none" },
  })

  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/source/project",
    currentWorktreePath: "/source/project",
    options: [{
      id: "existing:/source/project",
      kind: "existing",
      label: "project",
      path: "/source/project",
      branch: "main",
      isCurrent: true,
    }],
  })
  try {
    const decision = deriveWaitingRoomActivationDecision({
      state: { ...state, worktreeSelectionId: "existing:/source/project" },
      sessions: [],
      catalog: catalog(),
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
      remote: remote(),
    })
    assert.equal(decision.action, "create")
    if (decision.action === "create") {
      assert.equal(decision.launch.managedEnvironment?.kind, "new")
    }
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("Empty development does not serialize the selected Project", () => {
  const state: WaitingRoomState = {
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    managedComputeClass: "agent-small",
    managedRegion: "hel1",
    managedKernelContext: "empty",
    managedDevelopmentMode: "empty",
    projectSelectionId: "existing:project-1",
  }
  assert.deepEqual(managedEnvironmentContextPlanInput(state, remote()), {
    sourceTargetId: null,
    kernelContext: "empty",
    developmentSetup: { kind: "empty" },
    providerAccounts: { kind: "none" },
    gitCredentials: { kind: "none" },
  })
})

test("selected provider account uses the exact connected source", () => {
  const state: WaitingRoomState = {
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    managedComputeClass: "agent-small",
    managedRegion: "hel1",
    managedContextSourceTargetId: "source-target-1",
    managedProviderAccountSource: "selected_account",
    accountProfileId: "opencode-work",
  }
  assert.equal(managedEnvironmentDraftBlockReason(state, remote()), null)
  assert.deepEqual(managedEnvironmentContextPlanInput(state, remote()), {
    sourceTargetId: "source-target-1",
    kernelContext: "empty",
    developmentSetup: { kind: "empty" },
    providerAccounts: {
      kind: "selected",
      accounts: [{ provider: "opencode", accountProfile: "opencode-work" }],
    },
    gitCredentials: { kind: "none" },
  })
})

test("both Claude execution modes transfer the canonical Claude account", () => {
  for (const providerId of ["claude-headless", "claude-p"] as const) {
    let state = normalizeWaitingRoomState({
      ...baseState(),
      providerId,
      focus: "account",
      accountProfileId: "default",
      selectedMachineRef: NEW_MANAGED_MACHINE_REF,
      managedComputeClass: "agent-small",
      managedRegion: "hel1",
      managedContextSourceTargetId: "source-target-1",
      managedProviderAccountSource: "selected_account",
    }, [], catalog(), undefined, remote())
    state = cycleWaitingRoomValue(state, [], catalog(), 1, undefined, remote())
    assert.equal(state.accountProfileId, "claude-work")
    assert.equal(managedEnvironmentDraftBlockReason(state, remote()), null)
    assert.deepEqual(managedEnvironmentContextPlanInput(state, remote()).providerAccounts, {
      kind: "selected",
      accounts: [{ provider: "claude", accountProfile: "claude-work" }],
    })
  }
})

test("GitHub credential selection is source-bound and serialized", () => {
  const state: WaitingRoomState = {
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    managedComputeClass: "agent-small",
    managedRegion: "hel1",
    managedContextSourceTargetId: "source-target-1",
    managedGitCredentialSource: "selected",
  }
  assert.equal(managedEnvironmentDraftBlockReason(state, remote()), null)
  assert.deepEqual(managedEnvironmentContextPlanInput(state, remote()).gitCredentials, {
    kind: "selected",
    credentialIds: ["github"],
  })
  assert.equal(
    managedEnvironmentDraftBlockReason(state, { ...remote(), gitCredentials: [] }),
    "The connected source kernel has no transferable GitHub credential.",
  )
})

test("catalog refresh preserves explicit unavailable selections and blocks launch", () => {
  const normalized = normalizeWaitingRoomState({
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    managedComputeClass: "retired-class",
    managedRegion: "old-region",
    managedKernelContext: "source_kernel",
    managedContextSourceTargetId: "retired-source",
  }, [], catalog(), undefined, remote())

  assert.equal(normalized.managedComputeClass, "retired-class")
  assert.equal(normalized.managedRegion, "old-region")
  assert.equal(normalized.managedContextSourceTargetId, "retired-source")
  assert.equal(
    managedEnvironmentDraftBlockReason(normalized, remote()),
    "The selected managed compute class is unavailable.",
  )
  assert.equal(
    managedEnvironmentDraftBlockReason({
      ...normalized,
      managedComputeClass: "agent-small",
      managedRegion: "hel1",
    }, remote()),
    "The selected managed-context source is unavailable.",
  )
})

test("missing managed catalog never retargets an explicit managed launch to local", () => {
  const {
    managedComputeClasses: _computeClasses,
    managedContextSources: _contextSources,
    managedEnvironments: _environments,
    ...withoutManagedCatalog
  } = remote()
  const normalized = normalizeWaitingRoomState({
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
  }, [], catalog(), undefined, withoutManagedCatalog)

  assert.equal(normalized.selectedMachineRef, NEW_MANAGED_MACHINE_REF)
  assert.deepEqual(deriveWaitingRoomActivationDecision({
    state: normalized,
    sessions: [],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    remote: withoutManagedCatalog,
  }), {
    action: "error",
    message: "The selected managed compute class is unavailable.",
  })
})

test("custom auto-stop preserves an explicit never idle delay", () => {
  const normalized = normalizeWaitingRoomState({
    ...baseState(),
    managedAutoStopPreset: "custom",
    managedCustomMinimumRuntimeSeconds: 3600,
    managedCustomIdleDelaySeconds: null,
  }, [], catalog(), undefined, remote())

  assert.equal(normalized.managedCustomIdleDelaySeconds, null)
  assert.deepEqual(managedEnvironmentAutoStopPolicy(normalized), {
    minimumRuntimeSeconds: 3600,
    idleDelaySeconds: null,
  })
  assert.equal(cycleManagedCustomIdle({
    ...normalized,
    managedCustomIdleDelaySeconds: 3600,
  }, 1).managedCustomIdleDelaySeconds, null)
})

test("source-backed plans require the connected Machine and kernel", () => {
  const state: WaitingRoomState = {
    ...baseState(),
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    managedComputeClass: "agent-small",
    managedRegion: "hel1",
    managedKernelContext: "source_kernel",
    managedContextSourceTargetId: "source-target-1",
  }
  const wrongMachine = remote()
  wrongMachine.managedContextSources = (wrongMachine.managedContextSources ?? []).map((source) => ({
    ...source,
    machineId: "other-machine",
  }))
  assert.equal(
    managedEnvironmentDraftBlockReason(state, wrongMachine),
    "The selected managed-context source must be the connected kernel on this Machine.",
  )

  const disconnected = remote()
  const relay = disconnected.relay
  if (!relay) throw new Error("expected relay fixture")
  disconnected.relay = { ...relay, connected: false }
  assert.equal(
    managedEnvironmentDraftBlockReason(state, disconnected),
    "The selected managed-context source must be the connected kernel on this Machine.",
  )
})

function baseState(): WaitingRoomState {
  return createWaitingRoomState([], catalog(), "opencode", "opencode/gpt-5.4", "high")
}

function catalog() {
  return fallbackProviderCatalog()
}

function remote(): WaitingRoomRemoteState {
  return {
    workspaceId: "workspace-primary",
    worktreeId: "worktree-primary",
    relay: {
      configured: true,
      connected: true,
      daemon_id: "source-kernel",
      machine_id: "source-machine",
    },
    machines: [
      { machine_id: "managed-machine", kernel_count: 1 },
      { machine_id: "user-machine", kernel_count: 1 },
    ],
    kernels: [
      { kernel_id: "kernel-managed", machine_id: "managed-machine" },
      { kernel_id: "kernel-user", machine_id: "user-machine" },
    ],
    managedComputeClasses: [{ computeClass: "agent-small", regions: ["hel1", "fsn1"] }],
    managedContextSources: [{
      sourceTargetId: "source-target-1",
      machineId: "source-machine",
      kernelId: "source-kernel",
      label: "This kernel",
    }],
    managedEnvironments: [{
      environmentId: "environment-1",
      accountId: "account-1",
      createdByUserId: "user-1",
      name: "Managed build",
      region: "hel1",
      computeClass: "agent-small",
      desiredState: "running",
      observedState: "ready",
      desiredRevision: 1,
      observedRevision: 1,
      runtimeMachineId: "managed-machine",
      runtimeKernelId: "kernel-managed",
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
      contextManifestDigest: "sha256:context",
      autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
      lastErrorCode: null,
      lastErrorMessage: null,
      createdAt: "2026-08-21T00:00:00.000Z",
      updatedAt: "2026-08-21T00:00:00.000Z",
    }],
    projects: [{
      id: "project-1",
      owner_user_id: "user-1",
      workspace_id: "workspace-primary",
      workspace_ids: ["workspace-primary", "workspace-supporting"],
      name: "Project",
      kind: "named",
      status: "active",
      created_at_ms: 1,
      updated_at_ms: 2,
      session_count: 0,
      joined_collaborator_count: 0,
      pending_collaboration_invite_count: 0,
    }],
    providerAccounts: [{
      owner_user_id: "user-1",
      provider: "opencode",
      profile_id: "opencode-work",
      label: "OpenCode work",
      origin: "linked",
      is_default: false,
      auth_state: "authenticated",
      usage: {
        profile_id: "opencode-work",
        provider: "opencode",
        availability: "unavailable",
        source: "test",
      },
    }, {
      owner_user_id: "user-1",
      provider: "claude",
      profile_id: "default",
      label: "Claude default",
      origin: "linked",
      is_default: true,
      auth_state: "authenticated",
      usage: {
        profile_id: "default",
        provider: "claude",
        availability: "unavailable",
        source: "test",
      },
    }, {
      owner_user_id: "user-1",
      provider: "claude",
      profile_id: "claude-work",
      label: "Claude work",
      origin: "linked",
      is_default: false,
      auth_state: "authenticated",
      usage: {
        profile_id: "claude-work",
        provider: "claude",
        availability: "unavailable",
        source: "test",
      },
    }],
    gitCredentials: [{
      credentialId: "github",
      hostname: "github.com",
      label: "GitHub",
    }],
  }
}
