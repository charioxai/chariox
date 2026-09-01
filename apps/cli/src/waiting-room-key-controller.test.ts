import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { NEW_MANAGED_MACHINE_REF } from "./waiting-room-managed-environments.js"
import {
  createWaitingRoomKeyController,
  type WaitingRoomKeyControllerDeps,
} from "./waiting-room-key-controller.js"

test("waiting room key controller yields while attached or command center owns input", () => {
  const attachedHarness = createHarness({ attached: true })
  assert.equal(attachedHarness.controller.handleKey({ name: "enter" }), false)
  assert.deepEqual(attachedHarness.calls(), [])

  const commandHarness = createHarness({ commandCenterOpen: true })
  assert.equal(commandHarness.controller.handleKey({ name: "enter" }), false)
  assert.deepEqual(commandHarness.calls(), [])
})

test("waiting room key controller reconciles navigation keys", () => {
  const harness = createHarness()

  assert.equal(harness.controller.handleKey({ name: "down", eventType: "press" }), true)

  assert.equal(harness.reconciledStates().length, 1)
  assert.equal(harness.reconciledStates().at(-1)?.keyState.down, true)
  assert.deepEqual(harness.calls(), ["reconcile"])
})

test("waiting room key controller applies release navigation state", () => {
  const harness = createHarness()

  assert.equal(harness.controller.handleKey({ name: "down", eventType: "release" }), true)

  assert.equal(harness.updatedStates().length, 1)
  assert.equal(harness.updatedStates().at(-1)?.keyState.down, false)
  assert.deepEqual(harness.calls(), ["set-state", "rebuild"])
})

test("waiting room key controller dispatches lifecycle shortcuts", () => {
  const harness = createHarness()

  assert.equal(harness.controller.handleKey({ name: "a", eventType: "press" }), true)
  assert.equal(harness.controller.handleKey({ name: "d", eventType: "press" }), true)

  assert.deepEqual(harness.lifecycleActions(), ["archive", "delete"])
})

test("waiting room key controller activates on enter", () => {
  const harness = createHarness()

  assert.equal(harness.controller.handleKey({ name: "enter", eventType: "press" }), true)

  assert.deepEqual(harness.calls(), ["activate"])
})

test("waiting room key controller opens managed configuration from the Machine field", () => {
  const harness = createHarness({
    state: waitingRoomState({
      focus: "launch-machine",
      selectedMachineRef: NEW_MANAGED_MACHINE_REF,
    }),
  })

  assert.equal(harness.controller.handleKey({ name: "enter", eventType: "press" }), true)

  assert.deepEqual(harness.calls(), ["open-managed-machine"])
})

test("waiting room key controller opens managed configuration when Machine cycles to new", () => {
  const harness = createHarness({
    state: waitingRoomState({ focus: "launch-machine", selectedMachineRef: "local" }),
    remote: { managedEnvironments: [] },
  })

  assert.equal(harness.controller.handleKey({ name: "right", eventType: "press" }), true)

  assert.equal(harness.reconciledStates().at(-1)?.selectedMachineRef, NEW_MANAGED_MACHINE_REF)
  assert.deepEqual(harness.calls(), ["reconcile", "open-managed-machine"])
})

test("waiting room key controller renames and restores the focused project", () => {
  const project = projectSummary({ status: "archived" })
  const harness = createHarness({
    state: waitingRoomState({ focus: "project-entry", projectIndex: 0, showArchivedProjects: true }),
    projects: [project],
  })

  assert.equal(harness.controller.handleKey({ name: "e", eventType: "press" }), true)
  assert.equal(harness.controller.handleKey({ name: "r", eventType: "press" }), true)

  assert.deepEqual(harness.calls(), [
    `rename:${project.id}:${project.name}`,
    `restore:${project.id}`,
  ])
})

function createHarness(options: {
  attached?: boolean
  hotkeysOpen?: boolean
  promptFocused?: boolean
  commandCenterOpen?: boolean
  commandCenterQuery?: string
  state?: WaitingRoomState
  sessions?: SessionListEntry[]
  projects?: WaitingRoomProjectSummary[]
  remote?: WaitingRoomRemoteState
} = {}) {
  let state = options.state ?? waitingRoomState()
  const calls: string[] = []
  const reconciledStates: WaitingRoomState[] = []
  const updatedStates: WaitingRoomState[] = []
  const lifecycleActions: Array<Parameters<WaitingRoomKeyControllerDeps["applyLifecycleAction"]>[0]> = []

  const controller = createWaitingRoomKeyController({
    isAttached: () => options.attached ?? false,
    hotkeysOpen: () => options.hotkeysOpen ?? false,
    promptFocused: () => options.promptFocused ?? false,
    commandCenterOpen: () => options.commandCenterOpen ?? false,
    commandCenterQuery: () => options.commandCenterQuery ?? "",
    getWaitingRoomState: () => state,
    getSessions: () => options.sessions ?? [],
    getProviderCatalog: () => fallbackProviderCatalog(),
    getRemoteState: () => options.remote ?? (options.projects ? { projects: options.projects } : {}),
    reconcileWaitingRoom: (nextState) => {
      calls.push("reconcile")
      state = nextState
      reconciledStates.push(nextState)
    },
    setWaitingRoomState: (nextState) => {
      calls.push("set-state")
      state = nextState
      updatedStates.push(nextState)
    },
    rebuildTranscript: () => {
      calls.push("rebuild")
    },
    applyLifecycleAction: (action) => {
      calls.push(`lifecycle:${action}`)
      lifecycleActions.push(action)
    },
    beginProjectRename: (projectId, name) => {
      calls.push(`rename:${projectId}:${name}`)
    },
    restoreProject: (projectId) => {
      calls.push(`restore:${projectId}`)
    },
    activateWaitingRoom: () => {
      calls.push("activate")
    },
    openManagedMachineDialog: () => {
      calls.push("open-managed-machine")
      return true
    },
  })

  return {
    controller,
    calls: () => calls,
    reconciledStates: () => reconciledStates,
    updatedStates: () => updatedStates,
    lifecycleActions: () => lifecycleActions,
  }
}

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "current",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "medium",
    themeId: "system",
    introStep: 0,
    keyState: {
      up: false,
      down: false,
      left: false,
      right: false,
    },
    ...overrides,
  }
}

function projectSummary(overrides: Partial<WaitingRoomProjectSummary> = {}): WaitingRoomProjectSummary {
  return {
    id: "project-1",
    owner_user_id: "owner",
    workspace_id: "/workspace",
    name: "Workspace",
    kind: "default",
    status: "active",
    created_at_ms: 1,
    updated_at_ms: 2,
    session_count: 0,
    joined_collaborator_count: 0,
    pending_collaboration_invite_count: 0,
    ...overrides,
  }
}
