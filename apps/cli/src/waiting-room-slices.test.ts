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
  assert.equal(formatWaitingRoomSliceSelection("slice-1", slices), "linux-dev")
  assert.equal(formatWaitingRoomSliceSelection("none", slices), "None")
  assert.equal(formatWaitingRoomSliceSelection("new:headed", slices), "New headed")
  assert.equal(cycleWaitingRoomSliceSelectionId("none", slices, 1), "new:headless")
  assert.equal(cycleWaitingRoomSliceSelectionId("new:headless", slices, 1), "new:headed")
  assert.equal(cycleWaitingRoomSliceSelectionId("new:headed", slices, 1), "slice-1")
  assert.equal(cycleWaitingRoomSliceSelectionId("slice-1", slices, -1), "new:headed")
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
