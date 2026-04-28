import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomDeleteDecision,
  deriveWaitingRoomModelSelectionDecision,
  deriveWaitingRoomSessionLifecycleDecision,
  deriveWaitingRoomStateUpdate,
  deriveWaitingRoomVariantSelectionDecision,
} from "./waiting-room-controller.js"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState, type WaitingRoomState } from "./waiting-room.js"
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
      currentModel: "openai/gpt-5.4",
    })

    assert.deepEqual(decision, {
      action: "create",
      launch: {
        provider: "opencode",
        model: "openai/gpt-5.4",
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
