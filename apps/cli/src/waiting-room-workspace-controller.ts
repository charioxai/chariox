import { managedEnvironmentIdFromMachineRef, NEW_MANAGED_MACHINE_REF } from "@chariox/kernel-client/waiting-room-runtime-placement"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { getWorkspaceGitOverviewRequest, listWorkspaceWorktreesRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"
import {
  clearWaitingRoomWorktreeInventory,
  NO_REPOSITORY_WORKTREE_HINT,
  setWaitingRoomKernelWorktrees,
  UNBORN_WORKTREE_HINT,
  waitingRoomWorktreeDisabledHint,
} from "./waiting-room-worktrees.js"

export type WaitingRoomWorkspaceReadToken = {
  machineId: string
  kernelId: string
  path: string
  generation: number
}

type WorkspaceReadClient = { send<T>(request: unknown): Promise<T> }

export function waitingRoomWorkspaceSelection(
  state: Pick<WaitingRoomState, "selectedMachineRef" | "selectedKernelRef">,
  home: { machineId: string | null; kernelId: string | null },
  environments: readonly { environmentId: string; runtimeMachineId: string | null }[],
) {
  const environmentId = managedEnvironmentIdFromMachineRef(state.selectedMachineRef)
  const sourceSelection = state.selectedMachineRef === NEW_MANAGED_MACHINE_REF
  if (sourceSelection) return { machineId: home.machineId || "", kernelId: home.kernelId || "" }
  const machineId = state.selectedMachineRef === "local" ? home.machineId || ""
    : environmentId ? environments.find(environment => environment.environmentId === environmentId)?.runtimeMachineId || ""
    : state.selectedMachineRef || home.machineId || ""
  return {
    machineId,
    kernelId: state.selectedKernelRef === "local" ? home.kernelId || ""
      : state.selectedKernelRef || (machineId === home.machineId ? home.kernelId || "" : ""),
  }
}

export function createWaitingRoomWorkspaceController(deps: {
  getWorkspace(): string
  getWorktree(): string
  setWorkspace(path: string): void
  setWorktree(path: string): void
  resetSelection(): void
  send<T>(request: unknown): Promise<T>
  getSelection?(): { machineId: string; kernelId: string }
  withClient?(token: WaitingRoomWorkspaceReadToken, read: (client: WorkspaceReadClient) => Promise<void>): Promise<void>
  render(): void
  /** The waiting room is not mounted while a session is attached. */
  visible?(): boolean
}) {
  let machineId = ""
  let kernelId = ""
  let generation = 0
  let pendingMachineId: string | null = null
  let pending: WaitingRoomWorkspaceReadToken | null = null
  const workspaces = new Map<string, string>()
  const inventoryRequests = new WeakMap<WaitingRoomInventory, WaitingRoomWorkspaceReadToken>()
  // Periodic inventory refreshes must not rebuild (and drop selections in) an
  // attached transcript; leaving the session rebuilds the waiting room anyway.
  const render = () => { if (deps.visible?.() ?? true) deps.render() }

  function remember() {
    const path = deps.getWorkspace()
    const owner = pendingMachineId || machineId
    if (owner && path) workspaces.set(owner, path)
  }

  function selection() { return deps.getSelection?.() ?? { machineId: pendingMachineId || machineId, kernelId } }

  function captureInventoryRequest(): WaitingRoomWorkspaceReadToken {
    return { ...selection(), path: deps.getWorkspace(), generation }
  }

  async function readInventory(read: () => Promise<WaitingRoomInventory>) {
    const token = captureInventoryRequest()
    const inventory = await read()
    inventoryRequests.set(inventory, token)
    return inventory
  }

  function acceptsInventory(inventory: WaitingRoomInventory) {
    const token = inventoryRequests.get(inventory)
    const target = selection()
    return (!target.machineId || inventory.machineId === target.machineId && inventory.kernelId === target.kernelId)
      && (!token || token.generation === generation && token.path === deps.getWorkspace()
        && (!token.machineId || token.machineId === target.machineId)
        && (!token.kernelId || token.kernelId === target.kernelId))
  }

  function captureRequest(path = deps.getWorkspace()): WaitingRoomWorkspaceReadToken {
    const target = selection()
    const token = { ...target, path, generation: ++generation }
    pending = token
    return token
  }

  function isCurrent(token: WaitingRoomWorkspaceReadToken) {
    const target = selection()
    return token.generation === generation && token.path === deps.getWorkspace()
      && token.machineId === target.machineId && token.kernelId === target.kernelId
  }

  function finishRequest(token: WaitingRoomWorkspaceReadToken) {
    if (pending === token) pending = null
  }

  function beginMachineSelection(nextMachineId: string, nextKernelId = kernelId) {
    const ownerChanged = nextMachineId !== machineId || Boolean(pendingMachineId)
    if (!ownerChanged && nextKernelId === kernelId) return
    remember()
    generation += 1
    pending = null
    kernelId = nextKernelId
    if (ownerChanged) {
      pendingMachineId = nextMachineId
      clearWaitingRoomWorktreeInventory()
      const workspace = nextMachineId === machineId ? workspaces.get(machineId) || "" : ""
      deps.setWorkspace(workspace)
      deps.setWorktree(workspace)
      deps.resetSelection()
    }
    render()
  }

  async function refreshWorkspace(workspace = deps.getWorkspace(), client?: WorkspaceReadClient): Promise<void> {
    if (!workspace) return
    const token = captureRequest(workspace)
    const read = async (connection: WorkspaceReadClient) => {
      const send = async <T>(request: unknown): Promise<T> => {
        if (!isCurrent(token)) throw new Error("workspace read was superseded")
        return connection.send<T>(request)
      }
      const { worktrees, disabledHint } = await loadWaitingRoomKernelWorktrees(send, workspace)
      if (!isCurrent(token)) return
      setWaitingRoomKernelWorktrees(workspace, deps.getWorktree(), worktrees, disabledHint)
      render()
    }
    try {
      if (client) await read(client)
      else if (deps.withClient) await deps.withClient(token, read)
      else await read({ send: deps.send })
    } catch (error) {
      if (isCurrent(token)) throw error
    } finally {
      finishRequest(token)
    }
  }

  async function applyInventory(inventory: WaitingRoomInventory, send?: WorkspaceReadClient["send"]): Promise<void> {
    if (deps.getSelection && !acceptsInventory(inventory)) return
    if (pendingMachineId && pendingMachineId !== inventory.machineId) return
    const previousMachine = machineId
    remember()
    machineId = inventory.machineId
    kernelId = inventory.kernelId
    const machineChanged = Boolean(pendingMachineId || previousMachine && previousMachine !== machineId)
    pendingMachineId = null
    const latestSession = [...inventory.sessions].sort((a, b) =>
      (b.last_used_at_ms ?? b.created_at_ms) - (a.last_used_at_ms ?? a.created_at_ms))[0]
    const defaultWorkspace = inventory.launchTarget?.workspaceId || latestSession?.workspace_id || ""
    const workspace = machineChanged
      ? workspaces.get(machineId) || defaultWorkspace
      : deps.getWorkspace() || defaultWorkspace
    if (machineChanged || workspace !== deps.getWorkspace()) {
      clearWaitingRoomWorktreeInventory()
      deps.setWorkspace(workspace)
      deps.setWorktree(workspace === defaultWorkspace ? inventory.launchTarget?.worktreeId || workspace : workspace)
      deps.resetSelection()
    }
    remember()
    render()
    await refreshWorkspace(workspace, send ? { send } : undefined)
  }

  async function editWorkspace(path: string) {
    generation += 1
    pending = null
    deps.setWorkspace(path)
    deps.setWorktree(path)
    clearWaitingRoomWorktreeInventory()
    deps.resetSelection()
    remember()
    render()
    await refreshWorkspace(path)
  }

  return { applyInventory, beginMachineSelection, editWorkspace, refreshWorkspace,
    readInventory, acceptsInventory,
    captureRequest, isCurrent, finishRequest, isLoading: () => Boolean(pending && isCurrent(pending)) }
}

export async function loadWaitingRoomKernelWorktrees(
  send: <T>(request: unknown) => Promise<T>,
  workspace: string,
) {
  const response = await send<Record<string, unknown>>(listWorkspaceWorktreesRequest(workspace))
  const worktrees = expectVariant<{ worktrees: { path: string; branch?: string | null; label?: string | null; current: boolean }[] }>(response, "WorkspaceWorktreesListed").worktrees
  let disabledHint: string | null = null
  if (!worktrees.length) {
    const response = await send<Record<string, unknown>>(getWorkspaceGitOverviewRequest(workspace, workspace))
    const overview = expectVariant<{ overview: { repo_root?: string | null; compare_refs: { name: string }[] } }>(response, "WorkspaceGitOverview").overview
    disabledHint = !overview.repo_root ? NO_REPOSITORY_WORKTREE_HINT : overview.compare_refs.some(reference => reference.name === "HEAD") ? null : UNBORN_WORKTREE_HINT
  }
  return { worktrees, disabledHint }
}

export function createWaitingRoomWorkspacePlacementController(deps: {
  getState(): WaitingRoomState
  homeMachineId(): string | null
  managedEnvironments?(): readonly { environmentId: string; runtimeMachineId: string | null }[]
  beginMachineSelection(machineId: string, kernelId?: string): void
  connect(kernelRef: string, machineRef: string | undefined, isActive: () => boolean): Promise<boolean>
  browseManaged?(kernelRef: string, machineId: string, isActive: () => boolean): Promise<void>
  workspaceLoading?(): boolean
  homeKernelId?(): string | null
  refresh(): Promise<void>
  failure(error: unknown): void
}) {
  let browsingManagedWorkspace = false
  let placementGeneration = 0
  return {
    acceptsInventory(machineId: string, kernelId?: string) {
      const selected = waitingRoomWorkspaceSelection(deps.getState(), {
        machineId: deps.homeMachineId(), kernelId: deps.homeKernelId?.() || null,
      }, deps.managedEnvironments?.() ?? [])
      return (!selected.machineId || selected.machineId === machineId)
        && (!kernelId || !selected.kernelId || selected.kernelId === kernelId)
    },
    async refreshDisabledWorkspace() {
      // Ordinary snapshots recheck worktrees in applyInventory. Managed previews
      // keep the home control client, so recheck their filesystem separately.
      if (!browsingManagedWorkspace || deps.workspaceLoading?.() || !waitingRoomWorktreeDisabledHint()) return
      const { selectedMachineRef: machineRef, selectedKernelRef: kernelRef } = deps.getState()
      const environmentId = managedEnvironmentIdFromMachineRef(machineRef)
      const machineId = deps.managedEnvironments?.().find(environment => environment.environmentId === environmentId)?.runtimeMachineId
      if (!kernelRef || !machineId || !deps.browseManaged) return
      const requestedGeneration = placementGeneration
      const isActive = () => requestedGeneration === placementGeneration && deps.getState().selectedKernelRef === kernelRef
        && deps.getState().selectedMachineRef === machineRef
      try {
        await deps.browseManaged(kernelRef, machineId, isActive)
      } catch (error) {
        if (isActive()) deps.failure(error)
      }
    },
    select(state: WaitingRoomState) {
      const creatingMachine = state.selectedMachineRef === NEW_MANAGED_MACHINE_REF
      if (!state.selectedKernelRef && !creatingMachine) return
      const kernelRef = state.selectedKernelRef ?? ""
      const machineRef = state.selectedMachineRef
      const requestedGeneration = ++placementGeneration
      const environmentId = managedEnvironmentIdFromMachineRef(machineRef)
      // Creation has no destination kernel yet; its filesystem source is home.
      const source = waitingRoomWorkspaceSelection(state, {
        machineId: deps.homeMachineId(), kernelId: deps.homeKernelId?.() || null,
      }, deps.managedEnvironments?.() ?? [])
      const machineId = source.machineId
      if (machineId) deps.beginMachineSelection(machineId, source.kernelId || undefined)
      const isActive = () => requestedGeneration === placementGeneration && (deps.getState().selectedKernelRef ?? "") === kernelRef
        && deps.getState().selectedMachineRef === machineRef
      const browseManaged = environmentId && machineId ? deps.browseManaged : undefined
      browsingManagedWorkspace = Boolean(browseManaged)
      const connectHome = browseManaged || creatingMachine
      void deps.connect(connectHome ? "local" : kernelRef, connectHome ? "local" : machineRef, isActive).then(async connected => {
        if (!connected || !isActive()) return
        if (browseManaged) await browseManaged(kernelRef, machineId!, isActive)
        else await deps.refresh()
      }).catch(error => { if (isActive()) deps.failure(error) })
    },
  }
}
