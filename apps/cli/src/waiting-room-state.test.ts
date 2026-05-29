import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState, normalizeWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("waiting room state creation normalizes provider model, variant, and theme", () => {
  const catalog = fallbackProviderCatalog()
  const state = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high", "missing-theme")

  assert.equal(state.focus, "new")
  assert.equal(state.providerId, "opencode")
  assert.notEqual(state.modelId, "")
  assert.notEqual(state.effort, "")
  assert.equal(state.themeId, "opencode")
})

test("waiting room state normalization bounds indexes and redirects unavailable focus", () => {
  const catalog = fallbackProviderCatalog()
  const sessions = [
    session({ id: "one", last_used_at_ms: Date.UTC(2026, 0, 1) }),
    session({ id: "two", last_used_at_ms: Date.UTC(2026, 0, 2) }),
  ]

  const normalized = normalizeWaitingRoomState(
    waitingRoomState({
      focus: "session",
      sessionIndex: 5,
      machineIndex: 3,
      remoteKernelIndex: 2,
      terminalIndex: 4,
      sliceSelectionId: "slice-1",
    }),
    sessions,
    catalog,
    undefined,
    {
      slices: [slice()],
      machines: [{ machine_id: "machine-1", online: true, kernel_count: 1 }],
      kernels: [{ kernel_id: "kernel-1", machine_id: "machine-1" }],
      terminals: [{ terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 0, revoked: false }],
    },
  )

  assert.equal(normalized.focus, "session")
  assert.equal(normalized.sessionIndex, 1)
  assert.equal(normalized.machineIndex, 0)
  assert.equal(normalized.remoteKernelIndex, 0)
  assert.equal(normalized.terminalIndex, 0)
  assert.equal(normalized.sliceSelectionId, "slice-1")

  const withoutSessions = normalizeWaitingRoomState(
    waitingRoomState({ focus: "join-sessions", sessionIndex: 3 }),
    [],
    catalog,
  )
  assert.equal(withoutSessions.focus, "new")
  assert.equal(withoutSessions.sessionIndex, 0)
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
