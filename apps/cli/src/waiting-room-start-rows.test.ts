import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { catalogModelOptions, fallbackProviderCatalog } from "./provider-catalog.js"
import { waitingRoomStartRows } from "./waiting-room-start-rows.js"
import type { WaitingRoomState } from "./waiting-room.js"

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
    "provider",
    "model",
    "effort",
    "workspace",
    "worktree",
    "slice",
    "join-header",
  ])
  assert.equal(rows.find((row) => row.id === "provider")?.value, "OpenCode")
  assert.equal(rows.find((row) => row.id === "model")?.value, "GPT-5.4")
  assert.equal(rows.find((row) => row.id === "effort")?.value, "High")
  assert.equal(rows.find((row) => row.id === "workspace")?.value, "/workspace")
  assert.equal(rows.find((row) => row.id === "slice")?.value, "linux-dev")
  assert.equal(rows.find((row) => row.id === "join-header")?.value, "Press Enter")
  assert.equal(rows.find((row) => row.id === "join-header")?.focused, true)
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
  assert.equal(rows.find((row) => row.id === "slice")?.value, "loading..")
  assert.equal(rows.find((row) => row.id === "join-header")?.value, "loading..")
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "main",
    sliceSelectionId: "none",
    providerId: "opencode",
    modelId: "openai/gpt-5.4",
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
