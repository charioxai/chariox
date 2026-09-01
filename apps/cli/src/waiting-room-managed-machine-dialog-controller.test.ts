import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { createWaitingRoomManagedMachineDialogController } from "./waiting-room-managed-machine-dialog-controller.js"
import { NEW_MANAGED_MACHINE_REF } from "./waiting-room-managed-environments.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("managed-machine dialog opens on its first field and owns navigation until done", () => {
  let open = false
  let renders = 0
  let state = waitingRoomState()
  const controller = createWaitingRoomManagedMachineDialogController({
    isOpen: () => open,
    state: () => state,
    sessions: () => [],
    catalog: fallbackProviderCatalog,
    remote: () => ({}),
    setState: (next) => {
      state = next
    },
    openOverlay: () => {
      open = true
    },
    closeOverlay: () => {
      open = false
    },
    renderOverlay: () => {
      renders += 1
    },
  })

  assert.equal(controller.open(), true)
  assert.equal(open, true)
  assert.equal(state.focus, "managed-compute")

  assert.equal(controller.handleKey({ name: "down", eventType: "press" }), true)
  assert.equal(state.focus, "managed-region")
  assert.equal(renders, 1)

  assert.equal(controller.handleKey({ name: "enter", eventType: "press" }), true)
  assert.equal(open, false)
})

test("managed-machine dialog refuses to open for an ordinary Machine", () => {
  const state = waitingRoomState({ selectedMachineRef: "local" })
  const controller = createWaitingRoomManagedMachineDialogController({
    isOpen: () => false,
    state: () => state,
    sessions: () => [],
    catalog: fallbackProviderCatalog,
    remote: () => ({}),
    setState: () => undefined,
    openOverlay: () => assert.fail("must not open"),
    closeOverlay: () => undefined,
    renderOverlay: () => undefined,
  })

  assert.equal(controller.open(), false)
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "launch-machine",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "current",
    workspaceLiveSyncMode: "off",
    selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "medium",
    themeId: "system",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}
