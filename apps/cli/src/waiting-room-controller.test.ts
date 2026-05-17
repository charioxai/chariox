import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomControlActivationDecision,
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
      ...createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high"),
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
      ...createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high"),
      worktreeSelectionId: "create-worktree",
    }

    const decision = deriveWaitingRoomActivationDecision({
      state,
      sessions: [],
      catalog,
      currentProvider: "opencode",
      currentModel: "openai/gpt-5.4",
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

test("deriveWaitingRoomStateUpdate normalizes state and reports preference persistence", () => {
  const update = deriveWaitingRoomStateUpdate({
    currentState: waitingRoomState(),
    nextState: waitingRoomState({ sessionIndex: 7, effort: "invalid" }),
    sessions: [session("session-1"), session("session-2")],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "openai/gpt-5.4",
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
    currentModel: "openai/gpt-5.4",
  })
  const errorDecision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({ focus: "session" }),
    sessions: [],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "openai/gpt-5.4",
  })

  assert.deepEqual(joinDecision, {
    action: "join",
    session: sessions[0],
    launch: {
      provider: "opencode",
      model: "openai/gpt-5.4",
      effort: "high",
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

test("deriveWaitingRoomControlActivationDecision handles remote kernel activation", () => {
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
    action: "stage-command",
    command: "/relay cloud client-token builder-kernel",
    message: "press Enter to mint a relay token for builder-kernel",
  })
  assert.deepEqual(busy, {
    action: "error",
    message: "kernel busy-kernel is active",
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
    state: waitingRoomState({ focus: "add-terminal" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), { action: "open-terminal-pairing" })
  assert.deepEqual(deriveWaitingRoomControlActivationDecision({
    state: waitingRoomState({ focus: "join-sessions" }),
    workspacePath: "/workspace",
    worktreePath: "/workspace",
  }), { action: "open-session-browser" })
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

test("deriveWaitingRoomModelSelectionDecision validates models and normalizes variants", () => {
  const success = deriveWaitingRoomModelSelectionDecision({
    modelId: "anthropic/claude-sonnet-4",
    state: waitingRoomState(),
    sessions: [session("session-1")],
    catalog: catalog(),
    currentProvider: "opencode",
    configuredEffort: "high",
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
    assert.equal(success.selectedModelId, "anthropic/claude-sonnet-4")
    assert.equal(success.nextState.effort, "medium")
  }
  assert.deepEqual(failure, {
    kind: "error",
    message: "unknown model: missing/model",
  })
})

test("deriveWaitingRoomVariantSelectionDecision validates variants against the active model", () => {
  const success = deriveWaitingRoomVariantSelectionDecision({
    variant: "low",
    currentModelId: "openai/gpt-5.4",
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
    assert.equal(focused.nextState.focus, "provider")
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
  terminalIndex: 0,
    worktreeSelectionId: "existing:/workspace",
    providerId: "opencode",
    modelId: "openai/gpt-5.4",
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
        id: "openai",
        name: "OpenAI",
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
      openai: "gpt-5.4",
      anthropic: "claude-sonnet-4",
    },
    connected: ["codex", "openai", "anthropic"],
  }
}
