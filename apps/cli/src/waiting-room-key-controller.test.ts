import {createProjectEnvironmentReadController} from "./project-environment-read-controller.js"
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

test("a plain slash press focuses the waiting room prompt and leaves Enter activation alone", () => {
  const harness = createHarness()
  assert.equal(harness.controller.focusPromptOnSlash({ name: "/", eventType: "press" }), true)
  assert.deepEqual(harness.calls(), ["focus-prompt"])
  assert.equal(harness.controller.handleKey({ name: "enter" }), true)
  assert.deepEqual(harness.calls(), ["focus-prompt", "activate"])
})

test("slash entry ignores releases, modifiers, other keys, a focused prompt, drafts, dialogs and sessions", () => {
  const ignored: Array<[Parameters<ReturnType<typeof createHarness>["controller"]["focusPromptOnSlash"]>[0], Parameters<typeof createHarness>[0]]> = [
    [{ name: "/", eventType: "release" }, {}],
    [{ name: "/", ctrl: true }, {}],
    [{ name: "/", alt: true }, {}],
    [{ name: "k" }, {}],
    [{ name: "/" }, { promptFocused: true }],
    [{ name: "/" }, { promptFocused: true, commandCenterQuery: "/app" }],
    [{ name: "/" }, { commandCenterOpen: true }],
    [{ name: "/" }, { hotkeysOpen: true }],
    [{ name: "/" }, { attached: true }],
  ]
  for (const [event, options] of ignored) {
    const harness = createHarness(options)
    assert.equal(harness.controller.focusPromptOnSlash(event), false, JSON.stringify([event, options]))
    assert.deepEqual(harness.calls(), [])
  }
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

test("Machine navigation waits for Enter before opening managed configuration", () => {
  for (const name of ["right", "left"]) {
    const harness = createHarness({
      state: waitingRoomState({ focus: "launch-machine", selectedMachineRef: "local" }),
      remote: { managedEnvironments: [] },
    })
    assert.equal(harness.controller.handleKey({ name, eventType: "press" }), true)
    assert.equal(harness.reconciledStates().at(-1)?.selectedMachineRef, NEW_MANAGED_MACHINE_REF)
    assert.deepEqual(harness.calls(), ["reconcile"])
    assert.equal(harness.controller.handleKey({ name: "enter", eventType: "press" }), true)
    assert.deepEqual(harness.calls(), ["reconcile", "open-managed-machine"])
  }
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
  workspacePath?: string
  openProjectEnvironment?: WaitingRoomKeyControllerDeps["openProjectEnvironment"]
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
    getWorkspacePath: () => options.workspacePath ?? "/form/workspace",
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
    openProjectEnvironment: (projectId, workspaceId) => { calls.push(`environment:${projectId}${projectId === null ? `:${workspaceId}` : ""}`); options.openProjectEnvironment?.(projectId, workspaceId) },
    pinProject: (projectId, pinned) => { calls.push(`pin:${projectId}:${pinned}`) },
    beginNewProject: () => { calls.push("new-project-name") },
    beginProjectRename: (projectId, name) => {
      calls.push(`rename:${projectId}:${name}`)
    },
    restoreProject: (projectId) => {
      calls.push(`restore:${projectId}`)
    },
    focusPrompt: () => { calls.push("focus-prompt") },
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

test("MP-08 OWNER-15 P toggles a focused project pin", () => {
  const h = createHarness({ state: waitingRoomState({ focus: "project-entry", projectIndex: 0 }), projects: [projectSummary()] })
  h.controller.handleKey({ name: "p" })
  assert.ok(h.calls().includes(`pin:${projectSummary().id}:true`))
})

test("MP-08 OWNER-19 Enter on New asks for its name", () => {
  const h = createHarness({ state: waitingRoomState({ focus: "project", projectSelectionId: "new" }) })
  h.controller.handleKey({ name: "return" })
  assert.ok(h.calls().includes("new-project-name"))
})

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

// MP-08 / MP-10: No attached Session or Agent is required.
test("ENV P01 V opens exactly the selected Project and ignores release/modifiers", () => {
  const harness = createHarness({ state: waitingRoomState({ focus: "project-entry", projectIndex: 1 }), projects: [projectSummary(), projectSummary({ id: "second" })] })
  harness.controller.handleKey({ name: "v", eventType: "release" })
  harness.controller.handleKey({ name: "v", eventType: "press", ctrl: true })
  assert.deepEqual(harness.calls(), [])
  harness.controller.handleKey({ name: "v", eventType: "press" })
  assert.deepEqual(harness.calls(), ["environment:second"])
})

// MP-08 / MP-10 / MP-11: editable Workspace takes precedence over inventory defaults.
for (const workspaceId of ["/inventory/folder-a", undefined]) test(`Environment opens current form folder with inventory ${workspaceId}`,()=>{
  const harness=createHarness({state:waitingRoomState({focus:"workspace"}),remote:workspaceId ? {workspaceId} : {},workspacePath:"/selected/folder-b"})
  assert.equal(harness.controller.handleKey({name:"v",eventType:"press"}),true)
  assert.deepEqual(harness.calls(),["environment:null:/selected/folder-b"])
})

// MP-02 / MP-08 / MP-10 / MP-11: normal V with an empty Workspace cannot revive an old target.
test("review0440 Workspace V forgets a previous folder opening",async()=>{
 const requests:unknown[]=[]
 const environment=createProjectEnvironmentReadController({send:async request=>{requests.push(request);return {ProjectResolved:{project:null}}},render(){},pageSize:()=>100})
 await environment.open(null,"/previous");environment.close();requests.length=0
 let opening:Promise<void>|undefined
 const harness=createHarness({state:waitingRoomState({focus:"workspace"}),workspacePath:"",openProjectEnvironment:(id,workspace)=>{opening=environment.open(id,workspace)}})
 harness.controller.handleKey({name:"v",eventType:"press"});await opening
 assert(environment.visibleLines().some(line=>line.includes("Choose a folder")))
 environment.handleKey({name:"r"});await Promise.resolve()
 assert.equal(requests.length,0)
 environment.close()
})
