import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { catalogModelOptions, fallbackProviderCatalog } from "./provider-catalog.js"
import { waitingRoomStartRows } from "./waiting-room-start-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("waiting room start rows render configuration labels and join action", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const model = modelOptions[0] ?? null
  const rows = waitingRoomStartRows(
    waitingRoomState({ focus: "join-sessions", sliceSelectionId: "slice-1" }),
    { providerId: "opencode", model, effort: "high" },
    {
      modelOptions,
      remote: { slices: [slice()] },
      targets: { workspacePath: "/workspace", worktreePath: "/workspace" },
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 2,
      titleWidth: 24,
    },
  )

  assert.deepEqual(rows.map((row) => row.id), [
    "new",
    "launch-machine",
    "launch-kernel",
    "provider",
    "account",
    "model",
    "effort",
    "workspace",
    "worktree",
    "live-sync",
    "collaborators",
    "slice",
    "managed-development",
    "managed-repositories",
    "join-header",
  ])
  assert.equal(rows.find((row) => row.id === "launch-machine")?.value, "local")
  assert.equal(rows.find((row) => row.id === "launch-kernel")?.value, "local")
  assert.equal(rows.find((row) => row.id === "provider")?.value, "OpenCode")
  assert.equal(rows.find((row) => row.id === "account")?.value, "Default (not discovered)")
  assert.equal(rows.find((row) => row.id === "model")?.value, "GPT-5.4")
  assert.equal(rows.find((row) => row.id === "effort")?.value, "High")
  assert.equal(rows.find((row) => row.id === "workspace")?.value, "/workspace")
  assert.equal(rows.find((row) => row.id === "live-sync")?.value, "off (default; all repositories unrestricted)")
  assert.equal(rows.find((row) => row.id === "collaborators")?.value, "after session start")
  assert.equal(rows.find((row) => row.id === "slice")?.value, "linux-dev (running, headless, 0 agents, auth missing codex)")
  assert.equal(rows.find((row) => row.id === "managed-development")?.value, "Empty")
  assert.equal(rows.find((row) => row.id === "managed-repositories")?.value, "None")
  assert.equal(rows.find((row) => row.id === "join-header")?.value, "Press Enter")
  assert.equal(rows.find((row) => row.id === "join-header")?.focused, true)
})

test("waiting room start rows place project selection below Kernel", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const rows = waitingRoomStartRows(
    waitingRoomState({ projectSelectionId: "existing:project-1" }),
    { providerId: "opencode", model: modelOptions[0] ?? null, effort: "high" },
    {
      modelOptions,
      remote: {
        projects: [{
          id: "project-1",
          owner_user_id: "owner",
          workspace_id: "/workspace",
          name: "Frontend",
          kind: "named",
          status: "active",
          created_at_ms: 1,
          updated_at_ms: 2,
          session_count: 0,
          joined_collaborator_count: 0,
          pending_collaboration_invite_count: 0,
        }],
      },
      targets: { workspacePath: "/workspace", worktreePath: "/workspace" },
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows[2]?.id, "launch-kernel")
  assert.equal(rows[3]?.id, "project")
  assert.equal(rows[3]?.value, "Frontend")
})

test("waiting room start rows render loading placeholders before inventory arrives", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const rows = waitingRoomStartRows(
    waitingRoomState(),
    { providerId: "opencode", model: null, effort: "" },
    {
      modelOptions,
      inventoryLoading: true,
      loadingText: "loading..",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows.find((row) => row.id === "model")?.value, "No models available")
  assert.equal(rows.find((row) => row.id === "effort")?.value, "Default")
  assert.equal(rows.find((row) => row.id === "workspace")?.value, "loading..")
  assert.equal(rows.find((row) => row.id === "worktree")?.value, "loading..")
  assert.equal(rows.find((row) => row.id === "live-sync")?.value, "off (default; all repositories unrestricted)")
  assert.equal(rows.find((row) => row.id === "collaborators")?.value, "after session start")
  assert.equal(rows.find((row) => row.id === "slice")?.value, "loading..")
  assert.equal(rows.find((row) => row.id === "join-header")?.value, "loading..")
})

test("waiting room start rows hint cloud collaborator setup when cloud-linked", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const rows = waitingRoomStartRows(
    waitingRoomState({ focus: "collaborators" }),
    { providerId: "opencode", model: modelOptions[0] ?? null, effort: "high" },
    {
      modelOptions,
      remote: { collaborationBackend: "cloud" },
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows.find((row) => row.id === "collaborators")?.value, "use Cloud")
  assert.equal(rows.find((row) => row.id === "collaborators")?.focused, true)
})

test("waiting room start rows explain managed and tracked live sync modes", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")

  const offRows = waitingRoomStartRows(
    waitingRoomState({ workspaceLiveSyncMode: "off" }),
    { providerId: "opencode", model: modelOptions[0] ?? null, effort: "high" },
    {
      modelOptions,
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )
  const managedRows = waitingRoomStartRows(
    waitingRoomState({ workspaceLiveSyncMode: "managed" }),
    { providerId: "opencode", model: modelOptions[0] ?? null, effort: "high" },
    {
      modelOptions,
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )
  const trackedRows = waitingRoomStartRows(
    waitingRoomState({ workspaceLiveSyncMode: "tracked" }),
    { providerId: "opencode", model: modelOptions[0] ?? null, effort: "high" },
    {
      modelOptions,
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(offRows.find((row) => row.id === "live-sync")?.value, "off (default; all repositories unrestricted)")
  assert.equal(managedRows.find((row) => row.id === "live-sync")?.value, "managed (selected workspace/worktree only; other repositories unrestricted)")
  assert.equal(trackedRows.find((row) => row.id === "live-sync")?.value, "tracked (turn-end; selected workspace/worktree only; other repositories unrestricted)")
})

test("waiting room start rows mark provider choices from local fallback catalog", () => {
  const catalog = fallbackProviderCatalog({ source: "local_fallback" })
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const rows = waitingRoomStartRows(
    waitingRoomState(),
    {
      providerId: "opencode",
      model: modelOptions[0] ?? null,
      effort: "high",
      providerCatalogFallback: true,
    },
    {
      modelOptions,
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows.find((row) => row.id === "provider")?.value, "OpenCode (local list)")
  assert.equal(rows.find((row) => row.id === "model")?.value, "GPT-5.4 (local list)")
  assert.equal(rows.find((row) => row.id === "effort")?.value, "High (local list)")
})

test("waiting room start rows include selected slice provider account", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const rows = waitingRoomStartRows(
    waitingRoomState({ providerId: "opencode", sliceSelectionId: "slice-1" }),
    {
      providerId: "opencode",
      model: modelOptions[0] ?? null,
      effort: "high",
      slice: slice({
        providers: ["opencode"],
        provider_auth: [{ provider: "opencode:openai", account_profile: "default", state: "configured", account_id: "acct-1234567890abcdef", source: "slice" }],
      }),
    },
    {
      modelOptions,
      remote: { slices: [slice()] },
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows.find((row) => row.id === "provider")?.value, "OpenCode")
  assert.equal(rows.find((row) => row.id === "account")?.value, "Default (not discovered)")
})

test("waiting room account row shows only the public alias", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "codex")
  const rows = waitingRoomStartRows(
    waitingRoomState({ providerId: "codex" }),
    {
      providerId: "codex",
      model: modelOptions[0] ?? null,
      effort: "low",
      accountProfile: {
        owner_user_id: "local",
        provider: "codex",
        profile_id: "opaque-profile-id",
        label: "codex-2",
        origin: "linked",
        is_default: false,
        auth_state: "authenticated",
        identity_summary: "owner@example.com",
        usage: {
          profile_id: "opaque-profile-id",
          provider: "codex",
          availability: "available",
          source: "test",
          meters: [{
            meter_id: "credits",
            label: "Credits",
            kind: "credit_balance",
            scope: "account",
            state: "healthy",
            remaining: 42,
            source: "test",
            observed_at_ms: 1,
          }],
        },
      },
    },
    {
      modelOptions,
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows.find((row) => row.id === "account")?.value, "codex-2")
})

test("waiting room start rows do not conflate worker auth with the selected account", () => {
  const catalog = fallbackProviderCatalog()
  const modelOptions = catalogModelOptions(catalog, "opencode")
  const rows = waitingRoomStartRows(
    waitingRoomState({
      providerId: "opencode",
      selectedMachineRef: "machine-worker",
      selectedKernelRef: "kernel-worker",
    }),
    {
      providerId: "opencode",
      model: modelOptions[0] ?? null,
      effort: "high",
    },
    {
      modelOptions,
      remote: {
        machines: [{
          machine_id: "machine-worker",
          display_name: "worker",
          kernel_count: 1,
          available_providers: ["opencode"],
        }],
        kernels: [{
          kernel_id: "kernel-worker",
          machine_id: "machine-worker",
          available_providers: ["opencode"],
          provider_accounts: [{ provider: "opencode:openai", state: "configured", alias: "worker-openai" }],
        }],
      },
      inventoryLoading: false,
      loadingText: "loading",
      visibleSessionCount: 0,
      titleWidth: 24,
    },
  )

  assert.equal(rows.find((row) => row.id === "provider")?.value, "OpenCode")
  assert.equal(rows.find((row) => row.id === "account")?.value, "Default (not discovered)")
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "main",
    workspaceLiveSyncMode: "off",
    sliceSelectionId: "none",
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "high",
    themeId: "opencode",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
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
    ...overrides,
  }
}
