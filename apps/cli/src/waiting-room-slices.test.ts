import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import {
  cycleWaitingRoomSliceSelectionId,
  formatWaitingRoomSliceLabel,
  formatWaitingRoomSliceSelection,
  normalizeWaitingRoomSliceSelection,
  normalizeWaitingRoomSliceSelectionId,
  selectedWaitingRoomSliceRef,
  waitingRoomSelectedSlice,
  waitingRoomSliceOptions,
  waitingRoomSlices,
} from "./waiting-room-slices.js"
import { __setWaitingRoomWorktreeInventoryForTest } from "./waiting-room-worktrees.js"

test("waiting room slices sort by display label and project options", () => {
  const slices = waitingRoomSlices({
    slices: [
      slice({ id: "slice-z", name: "zeta" }),
      slice({ id: "slice-a", name: "" }),
      slice({ id: "slice-b", name: "beta" }),
    ],
  })

  assert.deepEqual(slices.map((entry) => formatWaitingRoomSliceLabel(entry)), ["beta", "slice-a", "zeta"])
  assert.deepEqual(waitingRoomSliceOptions(slices).map((option) => option.id), [
    "none",
    "new:headless",
    "new:headed",
    "slice-b",
    "slice-a",
    "slice-z",
  ])
})

test("waiting room slice selection normalizes ids and labels while preserving stale refs", () => {
  const slices = waitingRoomSlices({ slices: [slice({ id: "slice-1", name: "linux-dev" })] })

  assert.equal(normalizeWaitingRoomSliceSelectionId(" slice-1 ", slices), "slice-1")
  assert.equal(normalizeWaitingRoomSliceSelectionId("linux-dev", slices), "slice-1")
  assert.equal(normalizeWaitingRoomSliceSelectionId("deleted-slice", slices), "deleted-slice")
  assert.equal(normalizeWaitingRoomSliceSelectionId(null, slices), "none")
  assert.deepEqual(normalizeWaitingRoomSliceSelection("new:headed", "headless", slices), {
    sliceSelectionId: "new",
    sliceDisplayMode: "headed",
  })
  assert.deepEqual(normalizeWaitingRoomSliceSelection("new:headless", "headed", slices), {
    sliceSelectionId: "new",
    sliceDisplayMode: "headless",
  })
})

test("waiting room slice selection resolves refs, labels, and cycling", () => {
  const slices = waitingRoomSlices({
    slices: [
      slice({ id: "slice-1", name: "linux-dev" }),
      slice({ id: "slice-2", name: "mac-dev" }),
    ],
  })

  assert.equal(waitingRoomSelectedSlice("linux-dev", slices)?.id, "slice-1")
  assert.equal(selectedWaitingRoomSliceRef("slice-2", slices), "slice-2")
  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev (running, headless, 0 agents, auth missing codex)")
  assert.equal(formatWaitingRoomSliceSelection("none", slices), "off")
  assert.equal(formatWaitingRoomSliceSelection("new", slices), "new headless")
  assert.equal(formatWaitingRoomSliceSelection("new", slices, "headed"), "new headed")
  assert.equal(formatWaitingRoomSliceSelection("deleted-slice", slices), "reuse unavailable")
  assert.equal(cycleWaitingRoomSliceSelectionId("none", slices, 1), "new:headless")
  assert.equal(cycleWaitingRoomSliceSelectionId("new", slices, 1), "new:headed")
  assert.equal(cycleWaitingRoomSliceSelectionId("new:headed", slices, 1), "slice-1")
  assert.equal(cycleWaitingRoomSliceSelectionId("slice-1", slices, -1), "new:headed")
})

test("waiting room slice labels keep aliases and extracted auth identities visible", () => {
  const slices = waitingRoomSlices({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        agent_ids: ["agent-1", "agent-2", "agent-3", "agent-4"],
        display_mode: "headed",
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        provider_auth: [
          { provider: "codex", account_profile: "default", state: "configured", account_id: "acct-1", source: "slice" },
          { provider: "claude", account_profile: "default", state: "authenticated", email: "user@example.com", source: "slice" },
        ],
      }),
    ],
  })

  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev (running, headed, 4 agents: agent-1, agent-2, agent-3 +1 more, relay shared, codex default (acct-1), claude default (user@example.com))")
})

test("waiting room slice labels do not infer shared relay when private flag is missing", () => {
  const slices = waitingRoomSlices({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        relay_endpoint: { url: "wss://relay.example/slice" },
      }),
    ],
  })

  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev (running, headless, 0 agents, relay unknown, auth missing codex)")
})

test("waiting room slices filter reusable slices by selected worktree", () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [
      { id: "existing:/workspace", kind: "existing", label: "main", path: "/workspace", branch: "main", isCurrent: true },
      { id: "existing:/workspace-feature", kind: "existing", label: "feature", path: "/workspace-feature", branch: "feature", isCurrent: false },
      { id: "create-worktree", kind: "create", label: "Create worktree" },
    ],
  })

  try {
    const slices = waitingRoomSlices({
      slices: [
        slice({ id: "main-slice", name: "main", worktree_id: "/workspace" }),
        slice({ id: "feature-slice", name: "feature", worktree_id: "/workspace-feature", agent_ids: ["agent-1"] }),
      ],
    }, {
      workspacePath: "/workspace",
      worktreeSelectionId: "existing:/workspace-feature",
    })

    assert.deepEqual(slices.map((entry) => entry.id), ["feature-slice"])
    assert.equal(formatWaitingRoomSliceSelection("feature-slice", slices), "feature (running, headless, 1 agent: agent-1, auth missing codex)")
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room slices require the exact selected Project repository topology", () => {
  const remote = {
    workspaceId: "/primary",
    worktreeId: "/primary-worktree",
    projects: [{
      id: "project-1",
      owner_user_id: "local",
      workspace_id: "/primary",
      workspace_ids: ["/primary", "/supporting"],
      name: "Project",
      kind: "named" as const,
      status: "active" as const,
      created_at_ms: 1,
      updated_at_ms: 1,
      session_count: 1,
      joined_collaborator_count: 0,
      pending_collaboration_invite_count: 0,
    }],
    slices: [
      slice({
        id: "exact",
        development: {
          kind: "source_project",
          project_id: "project-1",
          repositories: [
            { role: "primary", workspaceId: "/primary", worktreeId: "/primary-worktree" },
            { role: "supporting", workspaceId: "/supporting", worktreeId: null },
          ],
        },
      }),
      slice({
        id: "stale",
        development: {
          kind: "source_project",
          project_id: "project-1",
          repositories: [
            { role: "primary", workspaceId: "/primary", worktreeId: "/primary-worktree" },
          ],
        },
      }),
      slice({
        id: "empty",
        development: { kind: "empty" },
      }),
      slice({ id: "legacy-empty" }),
    ],
  }
  const slices = waitingRoomSlices(remote, {
    workspacePath: "/primary",
    worktreePath: "/primary-worktree",
    projectSelectionId: "existing:project-1",
    developmentMode: "current_project",
  })
  assert.deepEqual(slices.map((entry) => entry.id), ["exact"])

  const primaryOnlySlices = waitingRoomSlices(remote, {
    workspacePath: "/primary",
    worktreePath: "/primary-worktree",
    projectSelectionId: "existing:project-1",
    developmentMode: "current_project",
    repositorySelection: {
      projectId: "project-1",
      primaryWorkspaceId: "/primary",
      supportingWorkspaceIds: [],
    },
  })
  assert.deepEqual(primaryOnlySlices.map((entry) => entry.id), ["stale"])

  const emptySlices = waitingRoomSlices(remote, {
    workspacePath: "/primary",
    worktreePath: "/primary-worktree",
    projectSelectionId: "existing:project-1",
    developmentMode: "empty",
  })
  assert.deepEqual(emptySlices.map((entry) => entry.id), ["empty", "legacy-empty"])

  const unresolvedProjectSlices = waitingRoomSlices(remote, {
    workspacePath: "/primary",
    worktreePath: "/primary-worktree",
    projectSelectionId: "new",
    developmentMode: "current_project",
  })
  assert.deepEqual(unresolvedProjectSlices, [])

  const missingPrimarySlices = waitingRoomSlices({
    worktreeId: remote.worktreeId,
    projects: remote.projects,
    slices: remote.slices,
  }, {
    worktreePath: "/primary-worktree",
    projectSelectionId: "existing:project-1",
    developmentMode: "current_project",
  })
  assert.deepEqual(missingPrimarySlices, [])
})

test("waiting room slice labels show partial and stale provider auth coverage", () => {
  const slices = waitingRoomSlices({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        providers: ["codex", "opencode", "claude"],
        provider_auth: [
          { provider: "codex", account_profile: "default", state: "configured", account_id: "acct-1", source: "slice" },
          { provider: "claude", account_profile: "default", state: "not_configured", source: "slice" },
        ],
      }),
    ],
  })

  assert.equal(
    formatWaitingRoomSliceSelection("slice-1", slices),
    "linux-dev (running, headless, 0 agents, codex default (acct-1), claude default (auth missing)/state=not_configured, missing opencode, refresh claude)",
  )
})

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
