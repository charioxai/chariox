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

test("waiting room launch ownership synchronizes derived target inventory without cancelling", () => {
  const initial = state({
    selectedMachineRef: "managed:environment:one",
    modelId: "gpt-5.6-luna",
    projectSelectionId: "existing:project-one",
  })
  const tracker = createWaitingRoomLaunchOwnershipTracker(initial)

  const projected = {
    ...initial,
    modelId: "gpt-5.6-sol",
    projectSelectionId: "default",
  }
  tracker.synchronize(projected)

  assert.equal(tracker.revision(), 0)

  tracker.update({ ...projected, projectSelectionId: "existing:project-two" })

  assert.equal(tracker.revision(), 1)
})

test("waiting room launch ownership tracks managed provider account selection", () => {
  const initial = state({
    selectedMachineRef: "managed:new",
    managedProviderAccountSource: "selected_account",
    managedProviderAccountSelection: [{ provider: "codex", accountProfile: "work" }],
  })
  const tracker = createWaitingRoomLaunchOwnershipTracker(initial)

  tracker.update({
    ...initial,
    managedProviderAccountSelection: [
      { provider: "codex", accountProfile: "work" },
      { provider: "claude", accountProfile: "default" },
    ],
  })

  assert.equal(tracker.revision(), 1)
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
