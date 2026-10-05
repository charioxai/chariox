import { managedEnvironmentIdFromMachineRef } from "@chariox/kernel-client/waiting-room-runtime-placement"
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

export function createWaitingRoomWorkspaceController(deps: {
  getWorkspace(): string
  setWorkspace(path: string): void
  setWorktree(path: string): void
  resetSelection(): void
  send<T>(request: unknown): Promise<T>
  render(): void
}) {
  let machineId = ""
  let generation = 0
  let pendingMachineId: string | null = null
  const workspaces = new Map<string, string>()

  function remember() {
    const path = deps.getWorkspace()
    if (machineId && path) workspaces.set(machineId, path)
  }

  function beginMachineSelection(nextMachineId: string) {
    if (nextMachineId === machineId && !pendingMachineId) return
    remember()
    pendingMachineId = nextMachineId
    generation += 1
    clearWaitingRoomWorktreeInventory()
    const workspace = nextMachineId === machineId ? workspaces.get(machineId) || "" : ""
    deps.setWorkspace(workspace)
    deps.setWorktree(workspace)
    deps.resetSelection()
    deps.render()
  }

  async function applyInventory(inventory: WaitingRoomInventory, send = deps.send): Promise<void> {
    if (pendingMachineId && pendingMachineId !== inventory.machineId) return
    const previousMachine = machineId
    remember()
    machineId = inventory.machineId
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
      deps.setWorktree(workspace)
      deps.resetSelection()
    }
    remember()
    deps.render()
    const requestedGeneration = ++generation
    if (!workspace) return
    const { worktrees, disabledHint } = await loadWaitingRoomKernelWorktrees(send, workspace)
    if (requestedGeneration !== generation || deps.getWorkspace() !== workspace || inventory.machineId !== machineId) return
    setWaitingRoomKernelWorktrees(workspace, workspace, worktrees, disabledHint)
    deps.render()
  }

  return { applyInventory, beginMachineSelection }
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

export async function refreshWaitingRoomKernelWorktrees(
  send: <T>(request: unknown) => Promise<T>,
  workspace: string,
  getWorkspace: () => string,
) {
  clearWaitingRoomWorktreeInventory()
  const { worktrees, disabledHint } = await loadWaitingRoomKernelWorktrees(send, workspace)
  if (getWorkspace() === workspace) setWaitingRoomKernelWorktrees(workspace, workspace, worktrees, disabledHint)
}

export function createWaitingRoomWorkspacePlacementController(deps: {
  getState(): WaitingRoomState
  homeMachineId(): string | null
  managedEnvironments?(): readonly { environmentId: string; runtimeMachineId: string | null }[]
  beginMachineSelection(machineId: string): void
  connect(kernelRef: string, machineRef: string | undefined, isActive: () => boolean): Promise<boolean>
  browseManaged?(kernelRef: string, machineId: string, isActive: () => boolean): Promise<void>
  refresh(): Promise<void>
  failure(error: unknown): void
}) {
  let browsingManagedWorkspace = false
  let refreshingDisabledWorkspace = false
  return {
    acceptsInventory(machineId: string) {
      const environmentId = managedEnvironmentIdFromMachineRef(deps.getState().selectedMachineRef)
      const selectedMachineId = environmentId
        ? deps.managedEnvironments?.().find(environment => environment.environmentId === environmentId)?.runtimeMachineId
        : null
      return !browsingManagedWorkspace || !selectedMachineId || selectedMachineId === machineId
    },
    async refreshDisabledWorkspace() {
      // Ordinary snapshots recheck worktrees in applyInventory. Managed previews
      // keep the home control client, so recheck their filesystem separately.
      if (!browsingManagedWorkspace || refreshingDisabledWorkspace || !waitingRoomWorktreeDisabledHint()) return
      const { selectedMachineRef: machineRef, selectedKernelRef: kernelRef } = deps.getState()
      const environmentId = managedEnvironmentIdFromMachineRef(machineRef)
      const machineId = deps.managedEnvironments?.().find(environment => environment.environmentId === environmentId)?.runtimeMachineId
      if (!kernelRef || !machineId || !deps.browseManaged) return
      const isActive = () => deps.getState().selectedKernelRef === kernelRef
        && deps.getState().selectedMachineRef === machineRef
      refreshingDisabledWorkspace = true
      try {
        await deps.browseManaged(kernelRef, machineId, isActive)
      } catch (error) {
        deps.failure(error)
      } finally {
        refreshingDisabledWorkspace = false
      }
    },
    select(state: WaitingRoomState) {
      if (!state.selectedKernelRef) return
      const kernelRef = state.selectedKernelRef
      const machineRef = state.selectedMachineRef
      const environmentId = managedEnvironmentIdFromMachineRef(machineRef)
      const machineId = machineRef === "local" ? deps.homeMachineId()
        : environmentId ? deps.managedEnvironments?.().find(environment => environment.environmentId === environmentId)?.runtimeMachineId
        : machineRef
      if (machineId) deps.beginMachineSelection(machineId)
      const isActive = () => deps.getState().selectedKernelRef === kernelRef
        && deps.getState().selectedMachineRef === machineRef
      const browseManaged = environmentId && machineId ? deps.browseManaged : undefined
      browsingManagedWorkspace = Boolean(browseManaged)
      void deps.connect(browseManaged ? "local" : kernelRef, browseManaged ? "local" : machineRef, isActive).then(async connected => {
        if (!connected || !isActive()) return
        if (browseManaged) await browseManaged(kernelRef, machineId!, isActive)
        else await deps.refresh()
      }).catch(deps.failure)
    },
  }
}
