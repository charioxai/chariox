import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import {
  cycleWaitingRoomSliceSelectionId,
  formatWaitingRoomSliceLabel,
  formatWaitingRoomSliceSelection,
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

test("waiting room slice selection normalizes ids without accepting labels", () => {
  const slices = waitingRoomSlices({ slices: [slice({ id: "slice-1", name: "linux-dev" })] })

  assert.equal(normalizeWaitingRoomSliceSelectionId(" slice-1 ", slices), "slice-1")
  assert.equal(normalizeWaitingRoomSliceSelectionId("linux-dev", slices), "none")
  assert.equal(normalizeWaitingRoomSliceSelectionId(null, slices), "none")
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
  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev (running, headless, 0 agents, auth missing)")
  assert.equal(formatWaitingRoomSliceSelection("none", slices), "off")
  assert.equal(formatWaitingRoomSliceSelection("new", slices), "new headless")
  assert.equal(formatWaitingRoomSliceSelection("new", slices, "headed"), "new headed")
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
        agent_ids: ["agent-1"],
        display_mode: "headed",
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        provider_auth: [
          { provider: "codex", state: "configured", alias: "work", account_id: "acct-1" },
          { provider: "claude", state: "authenticated", email: "user@example.com" },
        ],
      }),
    ],
  })

  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev (running, headed, 1 agent, relay shared, codex work (acct-1), claude user@example.com)")
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

  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev (running, headless, 0 agents, relay unknown, auth missing)")
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
    assert.equal(formatWaitingRoomSliceSelection("feature-slice", slices), "feature (running, headless, 1 agent, auth missing)")
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
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
