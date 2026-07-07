import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { waitingRoomAllSlices, waitingRoomSliceRows } from "./waiting-room-slice-rows.js"

test("waiting room slice rows list slices with lifecycle and auth context", () => {
  const rows = waitingRoomSliceRows({ focus: "slice-entry", sliceIndex: 0 }, {
    slices: [
      slice({ id: "slice-b", name: "beta" }),
      slice({
        id: "slice-a",
        name: "alpha",
        status: "running",
        display_mode: "headed",
        worktree_id: "/repo",
        agent_ids: ["agent-1", "agent-2", "agent-3", "agent-4"],
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        provider_auth: [{ provider: "codex", state: "configured", alias: "work", account_id: "acct-1", source: "slice" }],
      }),
    ],
  }, 16)

  assert.equal(rows[0]?.id, "slices-header")
  assert.equal(rows[0]?.value, "2 configured")
  assert.deepEqual(rows.slice(1).map((row) => row.id), ["slice:slice-a", "slice:slice-b"])
  assert.equal(rows[1]?.title, "alpha")
  assert.equal(rows[1]?.value, "running headed 4 agents: agent-1, agent-2, agent-3 +1 more relay shared /repo auth codex work (acct-1)")
  assert.equal(rows[2]?.value, "stopped headless 0 agents - auth missing codex")
  assert.equal(rows[1]?.focused, true)
  assert.equal(rows[1]?.selectable, true)
})

test("waiting room slice rows show partial provider auth coverage", () => {
  const rows = waitingRoomSliceRows({ focus: "slice-entry", sliceIndex: 0 }, {
    slices: [
      slice({
        id: "slice-a",
        name: "alpha",
        status: "running",
        providers: ["codex", "opencode", "claude"],
        provider_auth: [
          { provider: "codex", state: "configured", alias: "work", account_id: "acct-1", source: "slice" },
          { provider: "claude", state: "unknown", source: "slice" },
        ],
      }),
    ],
  }, 16)

  assert.equal(rows[1]?.value, "running headless 0 agents - auth codex work (acct-1),claude auth missing/state=unknown,missing opencode,refresh claude")
})

test("waiting room slice rows show empty and loading states", () => {
  assert.deepEqual(
    waitingRoomSliceRows({ focus: "new", sliceIndex: 0 }, { slices: [] }, 16).map((row) => [row.id, row.value]),
    [["slices-header", "0 configured"], ["slices-none", "none"]],
  )
  assert.deepEqual(
    waitingRoomSliceRows({ focus: "new", sliceIndex: 0 }, { inventoryStatus: "loading", loadingFrame: 2, slices: [] }, 16).map((row) => [row.id, row.value]),
    [["slices-header", "loading.."], ["slices-none", "loading.."]],
  )
})

test("waiting room all slices are sorted by display label", () => {
  assert.deepEqual(
    waitingRoomAllSlices({ slices: [slice({ name: "zeta" }), slice({ name: "alpha" })] }).map((entry) => entry.name),
    ["alpha", "zeta"],
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
    status: "stopped",
    display_mode: "headless",
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
