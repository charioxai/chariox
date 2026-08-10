import assert from "node:assert/strict"
import test from "node:test"

import type { ExternalProviderSessionRecord, SliceRecord } from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomControlActivationDecision,
  deriveWaitingRoomCreateSessionDecision,
  deriveWaitingRoomDeleteDecision,
  deriveWaitingRoomKeyNavigationDecision,
  deriveWaitingRoomModelSelectionDecision,
  deriveWaitingRoomSessionLifecycleDecision,
  deriveWaitingRoomStateUpdate,
  deriveWaitingRoomVariantSelectionDecision,
  waitingRoomSessionLifecycleActionForEvent,
} from "./waiting-room-controller.js"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import {
  __setWaitingRoomWorktreeInventoryForTest,
  resolvePendingWaitingRoomWorktreePath,
} from "./waiting-room-worktrees.js"

test("waiting room activation stages existing worktree selections for session creation", async () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [
      {
        id: "existing:/workspace",
        kind: "existing",
        label: "main",
        path: "/workspace",
        branch: "main",
        isCurrent: true,
      },
      {
        id: "existing:/workspace-feature",
        kind: "existing",
        label: "feature/ui",
        path: "/workspace-feature",
        branch: "feature/ui",
        isCurrent: false,
      },
      {
        id: "create-worktree",
        kind: "create",
        label: "Create worktree",
      },
    ],
  })

  try {
    const catalog = fallbackProviderCatalog()
    const state = {
      ...createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high"),
      worktreeSelectionId: "existing:/workspace-feature",
    }

    const decision = deriveWaitingRoomActivationDecision({
      state,
      sessions: [],
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
    })

    assert.deepEqual(decision, {
      action: "create",
      launch: {
        provider: "opencode",
        model: "opencode/gpt-5.4",
        effort: "high",
        ownerMachineRef: "local",
        ownerKernelRef: "local",
        workspaceLiveSyncMode: "off",
      },
    })
    assert.equal(
      await resolvePendingWaitingRoomWorktreePath("/workspace", "/workspace"),
      "/workspace-feature",
    )
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room activation opens focused unattached agents", () => {
  const catalog = fallbackProviderCatalog()
  const decision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({ focus: "external-session", externalSessionIndex: 0 }),
    sessions: [],
    catalog,
    currentProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    remote: {
      externalProviderSessions: [
        externalSession("codex:old", { last_modified_at_ms: 100 }),
        externalSession("claude:recent", { last_modified_at_ms: 200 }),
      ],
    },
  })

  assert.deepEqual(decision, {
    action: "import-external-session",
    externalSessionId: "claude:recent",
  })
})

test("waiting room activation stages create-worktree selections for session creation", async () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [
      {
        id: "existing:/workspace",
        kind: "existing",
        label: "main",
        path: "/workspace",
        branch: "main",
        isCurrent: true,
      },
      {
        id: "create-worktree",
        kind: "create",
        label: "Create worktree",
      },
    ],
  })

  try {
    const catalog = fallbackProviderCatalog()
    const state = {
      ...createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high"),
      worktreeSelectionId: "create-worktree",
    }

    const decision = deriveWaitingRoomActivationDecision({
      state,
      sessions: [],
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
    })

    assert.equal(decision.action, "create")
    assert.equal(
      await resolvePendingWaitingRoomWorktreePath("/workspace", "/workspace", {
        createWorktree: async () => "/workspace-created",
      }),
      "/workspace-created",
    )
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room session creation preserves Default, Existing, and New project policies", () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [{
      id: "existing:/workspace",
      kind: "existing",
      label: "main",
      path: "/workspace",
      branch: "main",
      isCurrent: true,
    }],
  })
  const project = {
    id: "project-1",
    owner_user_id: "owner",
    workspace_id: "/workspace",
    name: "Frontend",
    kind: "named" as const,
    status: "active" as const,
    created_at_ms: 1,
    updated_at_ms: 2,
    session_count: 0,
    joined_collaborator_count: 0,
    pending_collaboration_invite_count: 0,
  }
  const policies = [
    ["default", { kind: "default" }],
    ["existing:project-1", { kind: "existing", project_id: "project-1" }],
    ["new", { kind: "new" }],
  ] as const

  try {
    for (const [projectSelectionId, expected] of policies) {
      const decision = deriveWaitingRoomCreateSessionDecision({
        state: waitingRoomState({ projectSelectionId }),
        catalog: catalog(),
        currentProvider: "opencode",
        currentModel: "opencode/gpt-5.4",
        remote: { workspaceId: "/workspace", projects: [project] },
      })

      assert.equal(decision.action, "create")
      if (decision.action === "create") {
        assert.deepEqual(decision.launch.projectSelection, expected)
      }
    }
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room remote kernel selection creates a remote owner launch without worker placement", () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [{
      id: "existing:/workspace",
      kind: "existing",
      label: "main",
      path: "/workspace",
      branch: "main",
      isCurrent: true,
    }],
  })
  const catalog = fallbackProviderCatalog()
  try {
    const decision = deriveWaitingRoomActivationDecision({
      state: waitingRoomState({
        selectedMachineRef: "machine-1",
        selectedKernelRef: "kernel-1",
      }),
      sessions: [],
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
      remote: {
        machines: [{
          machine_id: "machine-1",
          machine_alias: "builder",
          display_name: "builder",
          trust_status: "approved",
          online: true,
          pending: false,
          kernel_count: 1,
          available_providers: ["opencode"],
        }],
        kernels: [{
          kernel_id: "kernel-1",
          machine_id: "machine-1",
          relay_alias: "builder-kernel",
          available_providers: ["opencode"],
          accepting_remote_leases: true,
          leased_agent_count: 0,
          local_session_count: 0,
        }],
      },
    })

    assert.equal(decision.action, "create")
    if (decision.action === "create") {
      assert.equal(decision.launch.ownerMachineRef, "machine-1")
      assert.equal(decision.launch.ownerKernelRef, "kernel-1")
      assert.equal(decision.launch.workerKernelRef ?? null, null)
      assert.equal(decision.launch.kernelRef ?? null, null)
    }
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room activation blocks stale reusable slice selections for new sessions", () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [
      {
        id: "existing:/workspace",
        kind: "existing",
        label: "main",
        path: "/workspace",
        branch: "main",
        isCurrent: true,
      },
    ],
  })
  const catalog = fallbackProviderCatalog()
  try {
    const state = {
      ...createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high"),
      sliceSelectionId: "deleted-slice",
    }

    const decision = deriveWaitingRoomActivationDecision({
      state,
      sessions: [],
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
      remote: { slices: [] },
    })
    const explicitCreate = deriveWaitingRoomCreateSessionDecision({
      state,
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
      remote: { slices: [] },
    })

    assert.deepEqual(decision, {
      action: "error",
      message: "Selected slice is unavailable for this worktree/kernel. Choose an available slice, new slice, or off.",
    })
    assert.deepEqual(explicitCreate, {
      action: "error",
      message: "Selected slice is unavailable for this worktree/kernel. Choose an available slice, new slice, or off.",
    })
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room activation creates regular sessions for slice and non-slice launches", () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [
      {
        id: "existing:/workspace",
        kind: "existing",
        label: "main",
        path: "/workspace",
        branch: "main",
        isCurrent: true,
      },
    ],
  })
  const catalog = fallbackProviderCatalog()
  try {
    const state = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high")
    const decision = deriveWaitingRoomActivationDecision({
      state,
      sessions: [],
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
    })

    assert.equal(decision.action, "create")
    if (decision.action === "create") {
      assert.equal("metaagent" in decision.launch, false)
    }

    const sliceDecision = deriveWaitingRoomCreateSessionDecision({
      state: {
        ...state,
        sliceSelectionId: "new",
      },
      catalog,
      currentProvider: "opencode",
      currentModel: "opencode/gpt-5.4",
    })
    assert.equal(sliceDecision.action, "create")
    if (sliceDecision.action === "create") {
      assert.equal("metaagent" in sliceDecision.launch, false)
      assert.deepEqual(sliceDecision.launch.sliceCreate, { displayMode: "headless" })
    }
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})


test("deriveWaitingRoomStateUpdate normalizes state and reports preference persistence", () => {
  const update = deriveWaitingRoomStateUpdate({
    currentState: waitingRoomState(),
    nextState: waitingRoomState({ sessionIndex: 7, effort: "invalid" }),
    sessions: [session("session-1"), session("session-2")],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
  })

  assert.equal(update.normalizedState.sessionIndex, 1)
  assert.equal(update.normalizedState.effort, "low")
  assert.equal(update.shouldPersistProviderPreferences, true)
})

test("deriveWaitingRoomActivationDecision returns join and error decisions for session focus", () => {
  const sessions = [session("session-1")]
  const joinDecision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({ focus: "session" }),
    sessions,
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
  })
  const errorDecision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({ focus: "session" }),
    sessions: [],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
  })

  assert.deepEqual(joinDecision, {
    action: "join",
    session: sessions[0],
    launch: {
      provider: "opencode",
      model: "opencode/gpt-5.4",
      effort: "high",
      ownerMachineRef: "local",
      ownerKernelRef: "local",
      workspaceLiveSyncMode: "off",
    },
  })
  assert.deepEqual(errorDecision, {
    action: "error",
    message: "no session available to join",
  })
})

test("deriveWaitingRoomControlActivationDecision stages workspace and worktree commands", () => {
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "workspace" }),
    workspacePath: "/repo",
    worktreePath: "/repo",
  }), {
    action: "stage-command",
    command: "/workspace /repo",
    message: "edit the workspace path and press Enter",
  })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "worktree" }),
    workspacePath: "/repo",
    worktreePath: "/repo-feature",
  }), {
    action: "stage-command",
    command: "/worktree /repo-feature",
    message: "edit the worktree path and press Enter",
  })
})

test("deriveWaitingRoomControlActivationDecision handles machine activation", () => {
  const approved = deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "machine" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      machines: [{
        machine_id: "machine-1",
        registry_alias: "builder",
        trust_status: "approved",
        online: true,
        pending: false,
        kernel_count: 2,
        available_providers: [],
      }],
    },
  })
  const inactive = deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "machine" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      machines: [{
        machine_id: "machine-2",
        display_name: "old-builder",
        trust_status: "approved",
        online: false,
        pending: false,
        kernel_count: 0,
        available_providers: [],
      }],
    },
  })

  assert.deepEqual(approved, {
    action: "stage-command",
    command: "/machine kernels builder",
    message: "press Enter to list kernels for builder",
  })
  assert.deepEqual(inactive, {
    action: "info",
    message: "press D twice to delete machine old-builder",
  })
})

test("deriveWaitingRoomControlActivationDecision browses remote kernel inventories", () => {
  const attachable = deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "remote-kernel" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      kernels: [{
        kernel_id: "kernel-1",
        machine_id: "machine-1",
        relay_alias: "builder-kernel",
        accepting_remote_leases: true,
        available_providers: ["codex"],
        leased_agent_count: 0,
        local_session_count: 0,
      }],
    },
  })
  const busy = deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "remote-kernel" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      kernels: [{
        kernel_id: "kernel-2",
        machine_id: "machine-1",
        relay_alias: "busy-kernel",
        accepting_remote_leases: false,
        leased_agent_count: 1,
        local_session_count: 0,
      }],
    },
  })

  assert.deepEqual(attachable, {
    action: "browse-kernel",
    kernelId: "kernel-1",
    machineId: "machine-1",
    label: "builder-kernel",
  })
  assert.deepEqual(busy, {
    action: "browse-kernel",
    kernelId: "kernel-2",
    machineId: "machine-1",
    label: "busy-kernel",
  })
})

test("deriveWaitingRoomControlActivationDecision handles terminal and dialog actions", () => {
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "relay" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), { action: "cloud" })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "terminal" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      terminals: [{
        terminal_id: "term-1",
        terminal_type: "web",
        paired_at_ms: 1,
        revoked: false,
      }],
    },
  }), {
    action: "info",
    message: "term-1 is a Web terminal",
  })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "slice-entry", sliceIndex: 0 }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      slices: [{
        id: "slice-1",
        name: "linux-dev",
        owner_kernel_id: "kernel-local",
        owner_machine_id: "machine-local",
        backend: "local_docker",
        os: "linux",
        status: "running",
        workspace_mount: null,
        worker_kernel_ref: "slice:slice-1",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        providers: ["codex"],
        display_endpoint: null,
        created_at_ms: 0,
        updated_at_ms: 0,
      }],
    },
  }), {
    action: "stage-command",
    command: "/slice status slice-1",
    message: "press Enter to show slice linux-dev",
  })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "live-sync" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), {
    action: "info",
    message: "Use left/right to choose off, managed, or tracked before starting the session. Live sync applies only to the selected workspace/worktree; other repositories stay unrestricted.",
  })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "add-terminal" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), { action: "open-terminal-pairing" })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "join-sessions" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), { action: "open-session-browser" })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "external-session-more" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), { action: "load-older-external-sessions" })
})

test("deriveWaitingRoomSessionLifecycleDecision selects focused sessions for archive and delete", () => {
  const sessions = [session("session-1"), session("session-2")]
  const archiveDecision = deriveWaitingRoomSessionLifecycleDecision({
    action: "archive",
    state: waitingRoomState({ focus: "session" }),
    sessions,
    catalog: catalog(),
  })
  const deleteDecision = deriveWaitingRoomSessionLifecycleDecision({
    action: "delete",
    state: waitingRoomState({ focus: "session", sessionIndex: 1 }),
    sessions,
    catalog: catalog(),
  })
  const archiveAllDecision = deriveWaitingRoomSessionLifecycleDecision({
    action: "archive",
    state: waitingRoomState({ focus: "join-sessions" }),
    sessions,
    catalog: catalog(),
  })
  const errorDecision = deriveWaitingRoomSessionLifecycleDecision({
    action: "delete",
    state: waitingRoomState({ focus: "new" }),
    sessions,
    catalog: catalog(),
  })

  assert.deepEqual(archiveDecision, {
    action: "archive",
    session: sessions[0],
  })
  assert.deepEqual(deleteDecision, {
    action: "delete",
    session: sessions[1],
  })
  assert.deepEqual(archiveAllDecision, {
    action: "archive-all",
    sessions,
  })
  assert.deepEqual(errorDecision, {
    action: "error",
    message: "select a session to delete",
  })
})

test("deriveWaitingRoomDeleteDecision deletes all sessions from join header", () => {
  const sessions = [session("session-1"), session("session-2")]
  const decision = deriveWaitingRoomDeleteDecision({
    state: waitingRoomState({ focus: "join-sessions" }),
    sessions,
    catalog: catalog(),
  })

  assert.deepEqual(decision, {
    action: "delete-all-sessions",
    sessions,
  })
})

test("deriveWaitingRoomDeleteDecision selects inactive machines and kernels", () => {
  const sessions = [session("session-1")]
  const machineDecision = deriveWaitingRoomDeleteDecision({
    state: waitingRoomState({ focus: "machine" }),
    sessions,
    catalog: catalog(),
    remote: {
      machines: [{
        machine_id: "machine-1",
        display_name: "builder",
        trust_status: "approved",
        online: false,
        pending: false,
        kernel_count: 0,
        available_providers: [],
      }],
    },
  })
  const kernelDecision = deriveWaitingRoomDeleteDecision({
    state: waitingRoomState({ focus: "remote-kernel" }),
    sessions,
    catalog: catalog(),
    remote: {
      kernels: [{
        kernel_id: "kernel-1",
        machine_id: "machine-1",
        relay_alias: "builder-kernel",
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
      }],
    },
  })
  const activeKernelDecision = deriveWaitingRoomDeleteDecision({
    state: waitingRoomState({ focus: "remote-kernel" }),
    sessions,
    catalog: catalog(),
    remote: {
      kernels: [{
        kernel_id: "kernel-2",
        machine_id: "machine-1",
        relay_alias: "busy-kernel",
        accepting_remote_leases: false,
        leased_agent_count: 1,
        local_session_count: 0,
      }],
    },
  })

  assert.deepEqual(machineDecision, {
    action: "delete-machine",
    machineId: "machine-1",
    label: "builder",
  })
  assert.deepEqual(kernelDecision, {
    action: "delete-kernel",
    kernelId: "kernel-1",
    label: "builder-kernel",
  })
  assert.deepEqual(activeKernelDecision, {
    action: "error",
    message: "kernel busy-kernel is active",
  })
})

test("deriveWaitingRoomDeleteDecision selects idle slices and blocks active slices", () => {
  const idleDecision = deriveWaitingRoomDeleteDecision({
    state: waitingRoomState({ focus: "slice-entry", sliceIndex: 0 }),
    sessions: [],
    catalog: catalog(),
    remote: {
      slices: [slice("slice-1", "linux-dev")],
    },
  })
  const activeDecision = deriveWaitingRoomDeleteDecision({
    state: waitingRoomState({ focus: "slice-entry", sliceIndex: 0 }),
    sessions: [],
    catalog: catalog(),
    remote: {
      slices: [slice("slice-2", "busy-dev", { agent_ids: ["agent-1", "agent-2"] })],
    },
  })

  assert.deepEqual(idleDecision, {
    action: "delete-slice",
    sliceId: "slice-1",
    label: "linux-dev",
  })
  assert.deepEqual(activeDecision, {
    action: "error",
    message: "slice busy-dev has 2 active agents",
  })
})

test("deriveWaitingRoomModelSelectionDecision validates models and normalizes variants", () => {
  const success = deriveWaitingRoomModelSelectionDecision({
    modelId: "opencode/gpt-5.4",
    state: waitingRoomState(),
    sessions: [session("session-1")],
    catalog: catalog(),
    currentProvider: "opencode",
    configuredEffort: "medium",
  })
  const failure = deriveWaitingRoomModelSelectionDecision({
    modelId: "missing/model",
    state: waitingRoomState(),
    sessions: [],
    catalog: catalog(),
    currentProvider: "opencode",
    configuredEffort: "high",
  })

  assert.equal(success.kind, "success")
  if (success.kind === "success") {
    assert.equal(success.selectedModelId, "opencode/gpt-5.4")
    assert.equal(success.nextState.effort, "high")
  }
  assert.deepEqual(failure, {
    kind: "error",
    message: "unknown model: missing/model",
  })
})

test("deriveWaitingRoomVariantSelectionDecision validates variants against the active model", () => {
  const success = deriveWaitingRoomVariantSelectionDecision({
    variant: "low",
    currentModelId: "opencode/gpt-5.4",
    currentProviderId: "opencode",
    state: waitingRoomState(),
    sessions: [],
    catalog: catalog(),
  })
  const failure = deriveWaitingRoomVariantSelectionDecision({
    variant: "high",
    currentModelId: "anthropic/claude-sonnet-4",
    currentProviderId: "opencode",
    state: waitingRoomState({ modelId: "anthropic/claude-sonnet-4", effort: "medium" }),
    sessions: [],
    catalog: catalog(),
  })

  assert.equal(success.kind, "success")
  if (success.kind === "success") {
    assert.equal(success.selectedVariant, "low")
    assert.equal(success.nextState.effort, "low")
  }
  assert.deepEqual(failure, {
    kind: "error",
    message: "unknown variant: high",
  })
})

test("deriveWaitingRoomKeyNavigationDecision moves focus and tracks release state", () => {
  const sessions = [session("session-1")]
  const focused = deriveWaitingRoomKeyNavigationDecision({
    event: { name: "down", eventType: "press" },
    state: waitingRoomState(),
    sessions,
    catalog: catalog(),
  })
  assert.equal(focused.action, "navigate")
  if (focused.action === "navigate") {
    assert.equal(focused.nextState.focus, "launch-machine")
    assert.equal(focused.nextState.keyState.down, true)
  }

  const released = deriveWaitingRoomKeyNavigationDecision({
    event: { name: "down", eventType: "release" },
    state: waitingRoomState({ keyState: { up: false, down: true, left: false, right: false } }),
    sessions,
    catalog: catalog(),
  })
  assert.equal(released.action, "release")
  if (released.action === "release") {
    assert.equal(released.nextState.keyState.down, false)
  }
})

test("deriveWaitingRoomKeyNavigationDecision cycles focused values", () => {
  const decision = deriveWaitingRoomKeyNavigationDecision({
    event: { name: "right", eventType: "press" },
    state: waitingRoomState({ focus: "provider" }),
    sessions: [],
    catalog: catalog(),
  })

  assert.equal(decision.action, "navigate")
  if (decision.action === "navigate") {
    assert.equal(decision.nextState.providerId, "codex")
    assert.equal(decision.nextState.keyState.right, true)
  }
})

test("deriveWaitingRoomKeyNavigationDecision cycles waiting-room live sync mode", () => {
  const decision = deriveWaitingRoomKeyNavigationDecision({
    event: { name: "right", eventType: "press" },
    state: waitingRoomState({ focus: "live-sync" }),
    sessions: [],
    catalog: catalog(),
  })

  assert.equal(decision.action, "navigate")
  if (decision.action === "navigate") {
    assert.equal(decision.nextState.workspaceLiveSyncMode, "managed")
  }
})

test("waitingRoomSessionLifecycleActionForEvent maps archive and delete keys", () => {
  assert.equal(waitingRoomSessionLifecycleActionForEvent({
    event: { name: "a", eventType: "press" },
    promptFocused: false,
  }), "archive")
  assert.equal(waitingRoomSessionLifecycleActionForEvent({
    event: { name: "delete", eventType: "press" },
    promptFocused: false,
  }), "delete")
  assert.equal(waitingRoomSessionLifecycleActionForEvent({
    event: { name: "d", eventType: "release" },
    promptFocused: false,
  }), null)
  assert.equal(waitingRoomSessionLifecycleActionForEvent({
    event: { name: "a", eventType: "press" },
    promptFocused: true,
  }), null)
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    sliceIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "existing:/workspace",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "high",
    themeId: "opencode",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}

function session(id: string): SessionListEntry {
  return {
    id,
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: "Created",
    created_at_ms: 1,
    attachment_ids: [],
  }
}

function externalSession(
  id: string,
  overrides: Partial<ExternalProviderSessionRecord> = {},
): ExternalProviderSessionRecord {
  return {
    external_session_id: id,
    provider: id.split(":")[0] ?? "codex",
    provider_session_id: id,
    title: "Imported task",
    title_source: "provider",
    first_prompt_preview: "Imported task",
    created_at_ms: 1,
    last_modified_at_ms: 2,
    capabilities: {
      can_read_history: true,
    },
    ...overrides,
  }
}

function slice(id: string, name: string, overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id,
    name,
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker",
    os: "linux",
    status: "stopped",
    workspace_mount: "/workspace",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    worker_kernel_ref: `slice:${id}`,
    worker_kernel_id: `kernel-${id}`,
    worker_machine_id: `machine-${id}`,
    providers: ["codex"],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function catalog(): ProviderCatalog {
  return {
    all: [
      {
        id: "codex",
        name: "Codex",
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: { medium: {}, high: {} },
          },
        },
      },
      {
        id: "opencode",
        name: "OpenCode Zen",
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: { low: {}, high: {} },
          },
        },
      },
      {
        id: "anthropic",
        name: "Anthropic",
        models: {
          "claude-sonnet-4": {
            id: "claude-sonnet-4",
            name: "Claude Sonnet 4",
            status: "active",
            variants: { medium: {} },
          },
        },
      },
    ],
    default: {
      codex: "gpt-5.4",
      opencode: "gpt-5.4",
      anthropic: "claude-sonnet-4",
    },
    connected: ["codex", "opencode", "anthropic"],
  }
}
