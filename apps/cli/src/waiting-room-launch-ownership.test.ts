import assert from "node:assert/strict"
import test from "node:test"

import { createWaitingRoomLaunchOwnershipTracker } from "./waiting-room-launch-ownership.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("waiting room launch ownership stays cancelled after selecting away and back", () => {
  const initial = state({ selectedMachineRef: "managed:environment:one" })
  const tracker = createWaitingRoomLaunchOwnershipTracker(initial)

  tracker.update({ ...initial, selectedMachineRef: "local" })
  tracker.update(initial)

  assert.equal(tracker.revision(), 2)
})

test("waiting room launch ownership ignores a derived kernel projection", () => {
  const initial = state({
    selectedMachineRef: "managed:environment:one",
    selectedKernelRef: "kernel-before",
  })
  const tracker = createWaitingRoomLaunchOwnershipTracker(initial)

  tracker.update({ ...initial, selectedKernelRef: "kernel-after" })

  assert.equal(tracker.revision(), 0)
})

function state(update: Partial<WaitingRoomState>): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "existing:/workspace",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "gpt-5.4",
    effort: "medium",
    themeId: "chariox",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...update,
  }
}
