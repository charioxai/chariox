import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { catalogModelOptions, fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { waitingRoomChoice, waitingRoomEfforts, waitingRoomModel } from "./waiting-room-choice.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

test("waiting room choice projects selected session, remote inventory, terminal, and slice", () => {
  const catalog = fallbackProviderCatalog()
  const sessions = [
    session({ id: "old", last_used_at_ms: Date.UTC(2026, 0, 1) }),
    session({ id: "new", last_used_at_ms: Date.UTC(2026, 0, 2) }),
  ]
  const remote = remoteState()
  const choice = waitingRoomChoice(
    waitingRoomState({
      sessionIndex: 1,
      machineIndex: 0,
      remoteKernelIndex: 0,
      terminalIndex: 0,
      sliceSelectionId: "slice-1",
    }),
    sessions,
    catalog,
    remote,
  )

  assert.equal(choice.session?.id, "old")
  assert.equal(choice.remoteMachine?.machine_id, "machine-1")
  assert.equal(choice.remoteKernel?.kernel_id, "kernel-1")
  assert.equal(choice.terminal?.terminal_id, "terminal-1")
  assert.equal(choice.slice?.id, "slice-1")
  assert.equal(choice.sliceRef, "slice-1")
})

test("waiting room model and effort helpers project provider model variants", () => {
  const catalog = fallbackProviderCatalog()
  const modelOption = catalogModelOptions(catalog, "opencode")[0]
  const model = waitingRoomModel(waitingRoomState({ modelId: modelOption?.id ?? "" }), catalog)

  assert.equal(model?.label, "GPT-5.4")
  assert.equal(waitingRoomEfforts(model).includes("high"), true)
  assert.deepEqual(waitingRoomEfforts(null), [""])
})

function remoteState(): WaitingRoomRemoteState {
  return {
    slices: [slice()],
    machines: [{
      machine_id: "machine-1",
      online: true,
      kernel_count: 1,
    }],
    kernels: [{
      kernel_id: "kernel-1",
      machine_id: "machine-1",
    }],
    terminals: [{
      terminal_id: "terminal-1",
      terminal_type: "cli",
      paired_at_ms: 0,
      revoked: false,
    }],
  }
}

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

function session(overrides: Partial<SessionListEntry> = {}): SessionListEntry {
  return {
    id: "session-1",
    alias: null,
    worktree_id: "/workspace/tree",
    status: "Active",
    created_at_ms: Date.UTC(2026, 0, 1, 9, 0),
    last_used_at_ms: Date.UTC(2026, 0, 1, 9, 0),
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
