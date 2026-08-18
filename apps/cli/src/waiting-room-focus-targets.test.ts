import assert from "node:assert/strict"
import test from "node:test"

import type { ExternalProviderSessionRecord, SliceRecord } from "./cli-types.js"
import type { SessionListEntry } from "./sessions.js"
import { moveWaitingRoomFocus, waitingRoomFocusTargets } from "./waiting-room-focus-targets.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

test("waiting room focus targets preserve menu order and sorted session indexes", () => {
  const sessions = [
    session({ id: "old", last_used_at_ms: Date.UTC(2026, 0, 1) }),
    session({ id: "new", last_used_at_ms: Date.UTC(2026, 0, 2) }),
  ]
  const targets = waitingRoomFocusTargets(sessions, remoteState())

  assert.deepEqual(targets.map((target) => target.focus), [
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
    "join-sessions",
    "session",
    "session",
    "relay",
    "machine",
    "remote-kernel",
    "slice-entry",
    "terminal",
    "add-terminal",
    "provider-accounts",
    "theme",
  ])
  assert.deepEqual(
    targets.filter((target) => target.focus === "session").map((target) => target.sessionIndex),
    [0, 1],
  )
  assert.deepEqual(
    targets.filter((target) => target.focus === "slice-entry").map((target) => target.sliceIndex),
    [0],
  )
})

test("waiting room focus movement tracks indexed targets", () => {
  const sessions = [
    session({ id: "old", last_used_at_ms: Date.UTC(2026, 0, 1) }),
    session({ id: "new", last_used_at_ms: Date.UTC(2026, 0, 2) }),
  ]
  let state = waitingRoomState({ focus: "join-sessions" })

  state = moveWaitingRoomFocus(state, sessions, 1)
  assert.equal(state.focus, "session")
  assert.equal(state.sessionIndex, 0)

  state = moveWaitingRoomFocus(state, sessions, 1)
  assert.equal(state.focus, "session")
  assert.equal(state.sessionIndex, 1)

  state = moveWaitingRoomFocus(state, sessions, 1)
  assert.equal(state.focus, "relay")
  assert.equal(state.sessionIndex, 1)

  state = waitingRoomState({ focus: "remote-kernel" })
  state = moveWaitingRoomFocus(state, sessions, 1, remoteState())
  assert.equal(state.focus, "slice-entry")
  assert.equal(state.sliceIndex, 0)
})

test("waiting room focus targets include unattached agents and pagination", () => {
  const sessions = [session({ id: "chariox-session" })]
  const targets = waitingRoomFocusTargets(sessions, {
    externalProviderSessions: [
      externalSession({ external_session_id: "codex:first" }),
      externalSession({ external_session_id: "claude:second" }),
    ],
    externalProviderSessionsHasMore: true,
    externalProviderSessionsNextCursor: "cursor-2",
  })

  assert.deepEqual(
    targets
      .filter((target) => target.focus === "session" || target.focus === "external-session" || target.focus === "external-session-more")
      .map((target) => ({
        focus: target.focus,
        sessionIndex: target.sessionIndex,
        externalSessionIndex: target.externalSessionIndex,
      })),
    [
      { focus: "session", sessionIndex: 0, externalSessionIndex: 0 },
      { focus: "external-session", sessionIndex: 0, externalSessionIndex: 0 },
      { focus: "external-session", sessionIndex: 0, externalSessionIndex: 1 },
      { focus: "external-session-more", sessionIndex: 0, externalSessionIndex: 0 },
    ],
  )
})

test("waiting room focus movement clamps stale unattached agent indexes", () => {
  const state = moveWaitingRoomFocus(
    waitingRoomState({ focus: "external-session", externalSessionIndex: 99 }),
    [],
    1,
    {
      externalProviderSessions: [
        externalSession({ external_session_id: "codex:old", last_modified_at_ms: 100 }),
        externalSession({ external_session_id: "opencode:middle", provider: "opencode", last_modified_at_ms: 200 }),
        externalSession({ external_session_id: "claude:recent", provider: "claude", last_modified_at_ms: 300 }),
      ],
      externalProviderSessionsHasMore: true,
      externalProviderSessionsNextCursor: "cursor-2",
    },
  )

  assert.equal(state.focus, "external-session-more")
  assert.equal(state.externalSessionIndex, 2)
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

function externalSession(overrides: Partial<ExternalProviderSessionRecord> = {}): ExternalProviderSessionRecord {
  return {
    external_session_id: "codex:first",
    provider: "codex",
    provider_session_id: "first",
    title: "External task",
    title_source: "provider",
    first_prompt_preview: "External task",
    created_at_ms: 1,
    last_modified_at_ms: 2,
    capabilities: {
      can_read_history: true,
    },
    ...overrides,
  }
}
