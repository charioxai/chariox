import { waitingRoomStartRows } from "./waiting-room-start-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { managedEnvironmentMachineRef, NEW_MANAGED_MACHINE_REF } from "./waiting-room-managed-environments.js"
import assert from "node:assert/strict"
import test from "node:test"
import { createWaitingRoomWorkspaceController, createWaitingRoomWorkspacePlacementController, waitingRoomWorkspaceSelection } from "./waiting-room-workspace-controller.js"
import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"
import { clearWaitingRoomWorktreeInventory, describeWaitingRoomWorktreeSelection, normalizeWaitingRoomWorktreeSelectionId, resolvePendingWaitingRoomWorktreePath, stageWaitingRoomWorktreeSelection, waitingRoomWorktreeDisabledHint, waitingRoomWorktreeOptions } from "./waiting-room-worktrees.js"

function inventory(machineId: string, path: string): WaitingRoomInventory {
  return { machineId, kernelId: `${machineId}-1`, sessions: [], launchTarget: { workspaceId: path, worktreeId: path } } as unknown as WaitingRoomInventory
}

function harness(repositoryState: "ready" | "not-repository" | "unborn" = "ready") {
  let workspace = "/Users/miguel/project"
  let worktree = workspace
  const controller = createWaitingRoomWorkspaceController({
    getWorkspace: () => workspace,
    getWorktree: () => worktree,
    setWorkspace: path => { workspace = path }, setWorktree: path => { worktree = path },
    resetSelection: () => {}, render: () => {},
    send: async <T>(request: unknown) => {
      if ("ListWorkspaceWorktrees" in (request as object)) {
        return { WorkspaceWorktreesListed: { worktrees: repositoryState === "ready" ? [{ path: workspace, branch: "main", current: true }] : [] } } as T
      }
      return { WorkspaceGitOverview: { overview: { repo_root: repositoryState === "not-repository" ? null : workspace, compare_refs: [] } } } as T
    },
  })
  return { controller, workspace: () => workspace, worktree: () => worktree, edit: (path: string) => { workspace = path }, setRepositoryState: (next: typeof repositoryState) => { repositoryState = next } }
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

for (const disabledState of ["not-repository", "unborn"] as const) {
  test(`TUI unchanged inventory rechecks ${disabledState} after git repair in the same workspace`, async () => {
    const h = harness(disabledState)
    try {
      const snapshot = inventory("mac", "/Users/miguel")
      await h.controller.applyInventory(snapshot)
      assert.ok(waitingRoomWorktreeDisabledHint())
      h.setRepositoryState("ready")
      await h.controller.applyInventory(snapshot)
      assert.equal(h.workspace(), "/Users/miguel/project")
      assert.equal(waitingRoomWorktreeDisabledHint(), null)
      assert.ok(waitingRoomWorktreeOptions().some(option => option.id === "create-worktree"))
    } finally { clearWaitingRoomWorktreeInventory() }
  })
}

for (const scenario of ["machine", "same-machine kernel", "identical paths", "selection ABA", "managed workspace edit"] as const) {
  for (const deferredRead of ["worktrees", "overview"] as const) {
    test(`TUI filesystem read interleaving: ${scenario}, deferred ${deferredRead}`, async () => {
      let selected = { machineId: "machine-a", kernelId: "kernel-a1" }
      let workspace = "/shared/repo"
      let defer = false
      let finish!: (response: unknown) => void
      const response = new Promise(resolve => { finish = resolve })
      const requests: Array<{ kernelId: string; request: Record<string, unknown> }> = []
      const closed: string[] = []
      const controller = createWaitingRoomWorkspaceController({
        getWorkspace: () => workspace, setWorkspace: path => { workspace = path }, setWorktree: () => {},
        getWorktree: () => workspace,
        resetSelection: () => {}, render: () => {}, getSelection: () => selected,
        send: async () => { throw new Error("must not read through home") },
        withClient: async (token, read) => {
          const kernelId = token.kernelId
          try {
            await read({ send: async <T>(request: unknown) => {
              const message = request as Record<string, unknown>
              requests.push({ kernelId, request: message })
              if ("ListWorkspaceWorktrees" in message) {
                if (defer && deferredRead === "worktrees") return await response as T
                return { WorkspaceWorktreesListed: { worktrees: defer && deferredRead === "overview" ? []
                  : [{ path: token.path, branch: `fresh-${kernelId}`, current: true }] } } as T
              }
              if (defer) return await response as T
              return { WorkspaceGitOverview: { overview: { repo_root: token.path, compare_refs: [{ name: "HEAD" }] } } } as T
            } })
          } finally { closed.push(kernelId) }
        },
      })
      try {
        await controller.applyInventory({ ...inventory("machine-a", workspace), kernelId: "kernel-a1" })
        defer = true
        const pending = controller.editWorkspace(workspace)
        await new Promise(resolve => setImmediate(resolve))
        assert.equal(controller.isLoading(), true)
        assert.equal(requests.at(-1)?.kernelId, "kernel-a1")
        defer = false
        if (scenario === "managed workspace edit") {
          // A managed preview retains home control; the edited path still reads kernel-a1.
          await controller.editWorkspace("/remote-only/repo")
        } else {
          const nextMachine = scenario === "same-machine kernel" ? "machine-a" : "machine-b"
          selected = { machineId: nextMachine, kernelId: nextMachine === "machine-a" ? "kernel-a2" : "kernel-b" }
          controller.beginMachineSelection(selected.machineId, selected.kernelId)
          assert.equal(controller.isLoading(), false)
          if (scenario === "selection ABA") {
            selected = { machineId: "machine-a", kernelId: "kernel-a1" }
            controller.beginMachineSelection(selected.machineId, selected.kernelId)
          }
          const nextPath = scenario === "machine" ? "/machine-b/repo" : "/shared/repo"
          await controller.applyInventory({ ...inventory(selected.machineId, nextPath), kernelId: selected.kernelId })
        }
        const ready = waitingRoomWorktreeOptions().map(option => ({ ...option }))
        assert.ok(ready.some(option => option.kind === "existing" && option.branch === `fresh-${selected.kernelId}`))
        finish(deferredRead === "worktrees"
          ? { WorkspaceWorktreesListed: { worktrees: [] } }
          : { WorkspaceGitOverview: { overview: { repo_root: null, compare_refs: [] } } })
        await pending
        assert.deepEqual(waitingRoomWorktreeOptions(), ready)
        assert.equal(waitingRoomWorktreeDisabledHint(), null)
        assert.equal(controller.isLoading(), false)
        assert.equal(closed.length, 3)
        assert.equal(requests.filter(call => "GetWorkspaceGitOverview" in call.request).length, deferredRead === "overview" ? 1 : 0)
      } finally { finish({ WorkspaceWorktreesListed: { worktrees: [] } }); clearWaitingRoomWorktreeInventory() }
    })
  }
}

for (const change of ["workspace edit", "kernel ABA", "machine ABA"] as const) {
  test(`TUI fences ordinary workspace snapshots across ${change}`, async () => {
    let selected = { machineId: "a", kernelId: "a-1" }
    let workspace = "/shared/repo"
    let reads = 0
    const controller = createWaitingRoomWorkspaceController({
      getWorkspace: () => workspace, setWorkspace: path => { workspace = path }, setWorktree: () => {},
      getWorktree: () => workspace,
      getSelection: () => selected, resetSelection: () => {}, render: () => {},
      send: async <T>() => { reads++; return { WorkspaceWorktreesListed: { worktrees: [{ path: workspace, branch: "fresh", current: true }] } } as T },
    })
    const old = inventory("a", workspace)
    let finish!: (snapshot: WaitingRoomInventory) => void
    const pending = controller.readInventory(() => new Promise(resolve => { finish = resolve }))
    try {
      if (change === "workspace edit") {
        // Even recommitting the same path invalidates the previous snapshot.
        await controller.editWorkspace(workspace)
      } else {
        selected = change === "kernel ABA" ? { machineId: "a", kernelId: "a-2" } : { machineId: "b", kernelId: "b-1" }
        controller.beginMachineSelection(selected.machineId, selected.kernelId)
        selected = { machineId: "a", kernelId: "a-1" }
        controller.beginMachineSelection(selected.machineId, selected.kernelId)
        workspace = "/shared/repo"
      }
      finish(old)
      await pending
      const previousReads = reads
      assert.equal(controller.acceptsInventory(old), false)
      await controller.applyInventory(old)
      assert.equal(reads, previousReads)
      assert.equal(controller.isLoading(), false)
      const fresh = await controller.readInventory(async () => ({ ...old }))
      assert.equal(controller.acceptsInventory(fresh), true)
    } finally { clearWaitingRoomWorktreeInventory() }
  })
}

test("TUI does not use the home kernel as a filesystem fallback for an unconnected managed machine", () => {
  const target = waitingRoomWorkspaceSelection({ selectedMachineRef: managedEnvironmentMachineRef("env") },
    { machineId: "home-machine", kernelId: "home-kernel" }, [{ environmentId: "env", runtimeMachineId: "remote-machine" }])
  assert.deepEqual(target, { machineId: "remote-machine", kernelId: "" })
})

test("TUI machine creation keeps source filesystem identity separate from the requested placement", () => {
  assert.deepEqual(waitingRoomWorkspaceSelection({ selectedMachineRef: NEW_MANAGED_MACHINE_REF },
    { machineId: "home-machine", kernelId: "home-kernel" }, []), { machineId: "home-machine", kernelId: "home-kernel" })
})

test("TUI uses the kernel current worktree when the pending path is absent from its inventory", async () => {
  let workspace = "/repo"
  let worktree = "/not-listed"
  const controller = createWaitingRoomWorkspaceController({
    getWorkspace: () => workspace, getWorktree: () => worktree,
    setWorkspace: path => { workspace = path }, setWorktree: path => { worktree = path },
    resetSelection: () => {}, render: () => {},
    send: async <T>() => ({ WorkspaceWorktreesListed: { worktrees: [
      { path: "/repo", branch: "main", current: false },
      { path: "/repo-feature", branch: "feature", current: true },
    ] } }) as T,
  })
  try {
    await controller.applyInventory(inventory("local", "/repo"))
    assert.equal(normalizeWaitingRoomWorktreeSelectionId(""), "existing:/repo-feature")
    assert.equal(normalizeWaitingRoomWorktreeSelectionId("existing:/repo"), "existing:/repo")
  } finally { clearWaitingRoomWorktreeInventory() }
})

test("TUI inventory defaults retain the kernel launch worktree when no CLI workspace was supplied", async () => {
  let workspace = ""
  let worktree = ""
  const controller = createWaitingRoomWorkspaceController({
    getWorkspace: () => workspace, getWorktree: () => worktree,
    setWorkspace: path => { workspace = path }, setWorktree: path => { worktree = path },
    resetSelection: () => {}, render: () => {},
    send: async <T>() => ({ WorkspaceWorktreesListed: { worktrees: [
      { path: "/repo", branch: "main", current: false },
      { path: "/repo-feature", branch: "feature", current: true },
    ] } }) as T,
  })
  try {
    await controller.applyInventory({ ...inventory("local", "/repo"),
      launchTarget: { workspaceId: "/repo", worktreeId: "/repo-feature" },
    })
    assert.equal(workspace, "/repo")
    assert.equal(worktree, "/repo-feature")
    assert.equal(normalizeWaitingRoomWorktreeSelectionId(""), "existing:/repo-feature")
  } finally { clearWaitingRoomWorktreeInventory() }
})

test("MP-08/MP-11 periodic inventory refresh never rebuilds an attached session's transcript", async () => {
  let visible = false
  let renders = 0
  const controller = createWaitingRoomWorkspaceController({
    getWorkspace: () => "/w", getWorktree: () => "/w", setWorkspace: () => {}, setWorktree: () => {},
    resetSelection: () => {}, render: () => { renders++ }, visible: () => visible,
    send: async <T>(request: unknown) => ("ListWorkspaceWorktrees" in (request as object)
      ? { WorkspaceWorktreesListed: { worktrees: [{ path: "/w", branch: "main", current: true }] } }
      : { WorkspaceGitOverview: { overview: { repo_root: "/w", compare_refs: [] } } }) as T,
  })
  try {
    await controller.applyInventory(inventory("mac", "/w"))
    assert.equal(renders, 0, "attached: inventory state updates without destroying the transcript")
    visible = true
    await controller.applyInventory(inventory("mac", "/w"))
    assert.ok(renders > 0)
  } finally { clearWaitingRoomWorktreeInventory() }
})
