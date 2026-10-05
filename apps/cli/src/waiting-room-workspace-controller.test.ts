import { waitingRoomStartRows } from "./waiting-room-start-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import assert from "node:assert/strict"
import test from "node:test"
import { createWaitingRoomWorkspaceController, createWaitingRoomWorkspacePlacementController } from "./waiting-room-workspace-controller.js"
import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"
import { clearWaitingRoomWorktreeInventory, describeWaitingRoomWorktreeSelection, resolvePendingWaitingRoomWorktreePath, stageWaitingRoomWorktreeSelection, waitingRoomWorktreeDisabledHint, waitingRoomWorktreeOptions } from "./waiting-room-worktrees.js"

function inventory(machineId: string, path: string): WaitingRoomInventory {
  return { machineId, kernelId: `${machineId}-1`, sessions: [], launchTarget: { workspaceId: path, worktreeId: path } } as unknown as WaitingRoomInventory
}

function harness(repositoryState: "ready" | "not-repository" | "unborn" = "ready") {
  let workspace = "/Users/miguel/project"
  let worktree = workspace
  const controller = createWaitingRoomWorkspaceController({
    getWorkspace: () => workspace,
    setWorkspace: path => { workspace = path }, setWorktree: path => { worktree = path },
    resetSelection: () => {}, render: () => {},
    send: async <T>(request: unknown) => {
      if ("ListWorkspaceWorktrees" in (request as object)) {
        return { WorkspaceWorktreesListed: { worktrees: repositoryState === "ready" ? [{ path: workspace, branch: "main", current: true }] : [] } } as T
      }
      return { WorkspaceGitOverview: { overview: { repo_root: repositoryState === "not-repository" ? null : workspace, compare_refs: [] } } } as T
    },
  })
  return { controller, workspace: () => workspace, worktree: () => worktree, edit: (path: string) => { workspace = path } }
}

test("TUI restores the selected machine's workspace across Mac Linux Mac and keeps same-machine paths", async () => {
  const h = harness()
  try {
    await h.controller.applyInventory(inventory("mac", "/Users/miguel"))
    h.controller.beginMachineSelection("linux")
    assert.equal(h.workspace(), "")
    await h.controller.applyInventory(inventory("linux", "/home/miguel"))
    assert.equal(h.workspace(), "/home/miguel")
    assert.equal(h.worktree(), "/home/miguel")
    h.edit("/home/miguel/repo")
    await h.controller.applyInventory({ ...inventory("linux", "/elsewhere"), kernelId: "linux-2" })
    assert.equal(h.workspace(), "/home/miguel/repo")
    h.controller.beginMachineSelection("mac")
    await h.controller.applyInventory(inventory("mac", "/Users/miguel"))
    assert.equal(h.workspace(), "/Users/miguel/project")
    h.controller.beginMachineSelection("linux")
    await h.controller.applyInventory(inventory("linux", "/home/miguel"))
    assert.equal(h.workspace(), "/home/miguel/repo")
  } finally { clearWaitingRoomWorktreeInventory() }
})

test("TUI cancels a deferred A to B selection when returning to A and restores inventory", async () => {
  const h = harness()
  let state = { selectedMachineRef: "mac", selectedKernelRef: "mac-1" } as WaitingRoomState
  let finishConnection!: () => void
  const connection = new Promise<void>(resolve => { finishConnection = resolve })
  const refreshed: string[] = []
  const placement = createWaitingRoomWorkspacePlacementController({
    getState: () => state, homeMachineId: () => "mac",
    beginMachineSelection: h.controller.beginMachineSelection,
    connect: async (_, machine, isActive) => {
      if (machine === "linux") await connection
      return isActive()
    },
    refresh: async () => {
      refreshed.push(state.selectedMachineRef!)
      await h.controller.applyInventory(inventory(state.selectedMachineRef!, "/Users/miguel"))
    },
    failure: error => { throw error },
  })
  try {
    await h.controller.applyInventory(inventory("mac", "/Users/miguel"))
    state = { ...state, selectedMachineRef: "linux", selectedKernelRef: "linux-1" }
    placement.select(state)
    assert.equal(h.workspace(), "")
    state = { ...state, selectedMachineRef: "mac", selectedKernelRef: "mac-1" }
    placement.select(state)
    assert.equal(h.workspace(), "/Users/miguel/project")
    assert.equal(h.worktree(), "/Users/miguel/project")
    finishConnection()
    await new Promise(resolve => setImmediate(resolve))
    assert.deepEqual(refreshed, ["mac"])
    assert.equal(waitingRoomWorktreeOptions()[0]?.id, "existing:/Users/miguel/project")
    await h.controller.applyInventory(inventory("mac", "/elsewhere"))
    assert.equal(h.workspace(), "/Users/miguel/project")
  } finally { finishConnection(); clearWaitingRoomWorktreeInventory() }
})

for (const repositoryState of ["not-repository", "unborn"] as const) {
  test(`TUI disables ${repositoryState} worktrees and session launch skips creation`, async () => {
    const h = harness(repositoryState)
    try {
      await h.controller.applyInventory(inventory("mac", "/Users/miguel"))
      assert.deepEqual(waitingRoomWorktreeOptions(), [])
      assert.match(waitingRoomWorktreeDisabledHint()!, repositoryState === "unborn" ? /no commits.*create a commit/ : /No git repository.*git init/)
      const rows = waitingRoomStartRows({ focus: "worktree", providerId: "opencode", worktreeSelectionId: "create-worktree" } as WaitingRoomState,
        { providerId: "opencode", model: null, effort: "" },
        { modelOptions: [], remote: {}, targets: { workspacePath: h.workspace(), worktreePath: h.workspace() }, inventoryLoading: false, loadingText: "loading", visibleSessionCount: 0, titleWidth: 24 })
      assert.equal(rows.find(row => row.id === "worktree")?.selectable, false)
      assert.equal(rows.find(row => row.id === "worktree")?.value, waitingRoomWorktreeDisabledHint())
      assert.equal(describeWaitingRoomWorktreeSelection("create-worktree"), waitingRoomWorktreeDisabledHint())
      assert.equal(stageWaitingRoomWorktreeSelection("create-worktree").ok, true)
      const path = await resolvePendingWaitingRoomWorktreePath(h.workspace(), h.workspace(), { createWorktree: async () => { throw new Error("must not create worktree") } })
      assert.equal(path, h.workspace())
    } finally { clearWaitingRoomWorktreeInventory() }
  })
}

test("TUI machine selection connects before refreshing and maps local back to the home machine", async () => {
  let state = { selectedMachineRef: "local", selectedKernelRef: "local" } as WaitingRoomState
  const calls: string[] = []
  const controller = createWaitingRoomWorkspacePlacementController({
    getState: () => state, homeMachineId: () => "mac",
    beginMachineSelection: machineId => { calls.push(`select:${machineId}`) },
    connect: async (kernelRef, machineRef, isActive) => { calls.push(`connect:${machineRef}:${kernelRef}`); return isActive() },
    refresh: async () => { calls.push("refresh") }, failure: error => { throw error },
  })
  controller.select(state)
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(calls, ["select:mac", "connect:local:local", "refresh"])
  calls.length = 0
  state = { ...state, selectedMachineRef: "linux", selectedKernelRef: "linux-1" }
  controller.select(state)
  state = { ...state, selectedMachineRef: "mac", selectedKernelRef: "mac-1" }
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(calls, ["select:linux", "connect:linux:linux-1"])
})

test("TUI creates the selected worktree through the connected kernel", async () => {
  const { createSession } = await import("./session-api.js")
  const { setWaitingRoomKernelWorktrees } = await import("./waiting-room-worktrees.js")
  const requests: unknown[] = []
  setWaitingRoomKernelWorktrees("/home/miguel/repo", "/home/miguel/repo", [{ path: "/home/miguel/repo", branch: "main", current: true }], null)
  try {
    stageWaitingRoomWorktreeSelection("create-worktree")
    const client = { send: async <T>(request: unknown) => {
      requests.push(request)
      if ("CreateWorkspaceWorktree" in (request as object)) return { WorkspaceWorktreeCreated: { worktree: { path: "/home/miguel/new-worktree" } } } as T
      return { SessionCreated: { session: { id: "session", workspace_id: "/home/miguel/repo", worktree_id: "/home/miguel/new-worktree", status: "Created", created_at_ms: 0, agents: [] } } } as T
    } } as unknown as import("./ipc.js").LocalIpcClient
    const session = await createSession(client, "/home/miguel/repo", "/home/miguel/repo")
    assert.equal(session.worktree_id, "/home/miguel/new-worktree")
    assert.equal((requests[0] as { CreateWorkspaceWorktree: { workspace_id: string } }).CreateWorkspaceWorktree.workspace_id, "/home/miguel/repo")
    assert.equal(requests.length, 2)
  } finally { clearWaitingRoomWorktreeInventory() }
})
