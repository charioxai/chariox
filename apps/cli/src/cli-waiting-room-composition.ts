import { acquireMachineAllocation, getMachineAllocation, machineAllocationCapability, type MachineAllocationCapability } from "./machine-allocation-api.js"
import { WaitingRoomMachineAllocationController } from "./waiting-room-machine-allocation-controller.js"
import { waitingRoomAllocationRemoteState } from "./waiting-room-machine-allocation.js"
import type { LocalIpcClient } from "./ipc.js"
import { normalizeProviderAccountProfileId } from "./waiting-room-provider-accounts.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { createWaitingRoomWorkspaceController, createWaitingRoomWorkspacePlacementController, waitingRoomWorkspaceSelection } from "./waiting-room-workspace-controller.js"
import type { CloudClient } from "./cloud-client.js"
import { createCloudWaitingRoomController } from "./cloud-waiting-room-controller.js"
import { randomUUID } from "node:crypto"
import { mergeExternalProviderSessionsSorted } from "@chariox/kernel-client/external-provider-sessions"
import { updateAgentConfig, updateAgentProfile } from "./agent-api.js"
import { createDetachedKernelConnectController } from "./detached-kernel-connect-controller.js"
import { importExternalProviderSession, listExternalProviderSessions } from "./external-provider-session-api.js"
import type { SliceRecord } from "./cli-types.js"
import {
  saveProviderPreferences,
  saveUiPreferences,
  mergeUiPreferences,
  relayCloudProfile,
} from "./preferences.js"
import { loadLocalKernelPresences } from "./local-kernel-presence.js"
import { createWaitingRoomKernelConnectionController } from "./waiting-room-kernel-client.js"
import {
  getProviderAuthStatus,
  getProviderCatalog,
  getProviderCommandCatalogs,
} from "./provider-api.js"
import { createProviderPromptProjectionController } from "./provider-prompt-projection-controller.js"
import { createProviderSelectionController } from "./provider-selection-controller.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { approveRemoteMachine, forgetRemoteMachine } from "./remote-machine-api.js"
import {
  archiveSessionById,
  createSession,
  deleteSessionByRef,
} from "./session-api.js"
import type { SessionListEntry } from "./sessions.js"
import {
  createSlice,
  deleteSlice,
  startSlice,
} from "./slice-api.js"
import { applyTheme } from "./theme.js"
import {
  createWaitingRoomActivationController,
} from "./waiting-room-activation-controller.js"
import { cliWaitingRoomSliceApiOptions } from "./waiting-room-slice-api-options.js"
import type { WaitingRoomLaunchConfig } from "./waiting-room-controller.js"
import { getWaitingRoomInventory } from "./waiting-room-inventory-api.js"
import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import {
  createWaitingRoomInventoryCache,
  waitingRoomInventoryCacheScopeKey,
} from "./waiting-room-inventory-cache.js"
import { createWaitingRoomLifecycleActionController } from "./waiting-room-lifecycle-action-controller.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import { createWaitingRoomReconcileController } from "./waiting-room-reconcile-controller.js"
import {
  archiveProject,
  deleteProject,
  renameProject,
  restoreProject,
} from "./project-api.js"
import {
  clearStagedWaitingRoomWorktreeSelection,
} from "./waiting-room-worktrees.js"
import { existingProjectSelectionId } from "./waiting-room-projects.js"
import {
  type MutableLocalIpcClientPivot,
} from "./mutable-local-ipc-client.js"

type AnyFn = (...args: any[]) => any

export type CliWaitingRoomCompositionDeps = {
  cloudClient?: CloudClient
  onLaunchRefused?: (message: string, retry: () => void) => void
  client: any
  options: any
  appLogger: any
  formatError: AnyFn
  isAttached: AnyFn
  kernelConnected: AnyFn
  waitingRoomState: AnyFn
  setWaitingRoomState: AnyFn
  setWaitingRoomStateProjection: AnyFn
  waitingRoomLaunchOwnershipRevision: AnyFn
  availableSessions: AnyFn
  setAvailableSessions: AnyFn
  waitingRoomProjects: AnyFn
  setWaitingRoomProjects: AnyFn
  providerCatalogState: AnyFn
  setProviderCatalogState: AnyFn
  providerCommandCatalogState: AnyFn
  setProviderCommandCatalogState: AnyFn
  themeRegistryState: AnyFn
  waitingRoomCloudNotice: AnyFn
  waitingRoomInventoryStatus: AnyFn
  setWaitingRoomInventoryStatus: AnyFn
  waitingRoomHiddenKernelController: {
    hideKernel: AnyFn
    isKernelHidden: AnyFn
  }
  relayStatusState: AnyFn
  setRelayStatusState: AnyFn
  remoteMachinesState: AnyFn
  setRemoteMachinesState: AnyFn
  remoteKernelsState: AnyFn
  setRemoteKernelsState: AnyFn
  setProviderAccountCopyOffersState?: AnyFn
  providerAccountsState: AnyFn
  setProviderAccountsState: AnyFn
  terminalsState: AnyFn
  setTerminalsState: AnyFn
  slicesState: AnyFn
  setSlicesState: AnyFn
  externalProviderSessionsState: AnyFn
  setExternalProviderSessionsState: AnyFn
  externalProviderSessionsPageState: AnyFn
  setExternalProviderSessionsPageState: AnyFn
  pendingWorkspaceTarget: AnyFn
  setPendingWorkspaceTarget: AnyFn
  pendingWorktreeTarget: AnyFn
  setPendingWorktreeTarget: AnyFn
  preferencesState: AnyFn
  setPreferencesState: AnyFn
  setThemeRevision: AnyFn
  resetTranscriptSyntax: AnyFn
  applyResponseLayout: AnyFn
  renderCommandCenter: AnyFn
  rebuildTranscript: AnyFn
  updateSessionChrome: AnyFn
  syncCommandCenter: AnyFn
  handleCloudCommand: AnyFn
  setPromptText: AnyFn
  focusPrompt: AnyFn
  openTerminalPairingDialog: AnyFn
  openSessionBrowserDialog: AnyFn
  attachBinding: AnyFn
  rollbackAttachedSession: AnyFn
  flashFooter: AnyFn
  setKernelConnected: AnyFn
  setDaemonDisconnected: AnyFn
  sessionBrowserOpen: AnyFn
  closeSessionBrowserDialog: AnyFn
  focusedProviderRun: AnyFn
  focusedAgent: AnyFn
  focusedAgentId: AnyFn
  providerRunState: AnyFn
  sessionState: AnyFn
  applySessionState: AnyFn
  setProviderRunState: AnyFn
  appendNotice: AnyFn
}

export function createCliWaitingRoomComposition(deps: CliWaitingRoomCompositionDeps) {
  const waitingRoomInventoryCacheScope = () => (
    waitingRoomInventoryCacheScopeKey(relayCloudProfile(deps.preferencesState()))
  )
  const waitingRoomInventoryCache = createWaitingRoomInventoryCache(
    undefined,
    undefined,
    undefined,
    waitingRoomInventoryCacheScope,
  )
  const cachedWaitingRoomInventories = waitingRoomInventoryCache.load()
  let providerCatalogSelectionRevision = 0
  let manualKernelSelectionGeneration = 0
  let allocationCapability: MachineAllocationCapability | null | undefined
  let homeKernelId: string | null = null
  let homeMachineId: string | null = null
  let sourceLaunchTarget: { workspaceId: string; worktreeId: string } | null | undefined
  const managedWaitingRoomRemote = () => ({
    ...(sourceLaunchTarget
      ? { workspaceId: sourceLaunchTarget.workspaceId, worktreeId: sourceLaunchTarget.worktreeId }
      : {}),
    ...waitingRoomAllocationRemoteState(allocationCapability),
  })
  let workspacePlacementController: ReturnType<typeof createWaitingRoomWorkspacePlacementController> | undefined
  const kernelConnectionController = createWaitingRoomKernelConnectionController({
    cloudClient: deps.cloudClient, kernelConnected: deps.kernelConnected,
    getInventory: client => getInventoryForClient(client),
    client: deps.client,
    clientId: deps.options.clientId,
    initialTargetKernelId: deps.options.targetDaemonId,
    homeKernelId: () => homeKernelId,
    homeMachineId: () => homeMachineId,
    currentKernelId: () => deps.relayStatusState()?.daemon_id?.trim(),
    invalidateInventory: () => waitingRoomInventoryRefreshController.invalidate(),
    connected: (label) => {
      deps.setKernelConnected(true)
      deps.setDaemonDisconnected(false)
      deps.flashFooter(`connected to kernel ${label}`, "info")
    },
  })
  const waitingRoomWorkspaceController = createWaitingRoomWorkspaceController({
    getWorkspace: deps.pendingWorkspaceTarget,
    getWorktree: deps.pendingWorktreeTarget,
    setWorkspace: deps.setPendingWorkspaceTarget,
    setWorktree: deps.setPendingWorktreeTarget,
    resetSelection: () => deps.setWaitingRoomStateProjection({ ...deps.waitingRoomState(), worktreeSelectionId: "", projectSelectionId: "default" }),
    send: (request) => deps.client.send(request),
    getSelection: () => waitingRoomWorkspaceSelection(deps.waitingRoomState(), { machineId: homeMachineId, kernelId: homeKernelId }),
    withClient: (token, read) => kernelConnectionController.readWorkspace({
      ...token, isActive: () => waitingRoomWorkspaceController.isCurrent(token),
    }, read),
    render: deps.rebuildTranscript,
    visible: () => !deps.isAttached(),
  })
  const waitingRoomReconcileController = createWaitingRoomReconcileController({
    getCurrentState: deps.waitingRoomState,
    placementChanged: (state) => workspacePlacementController?.select(state),
    setWaitingRoomState: deps.setWaitingRoomState,
    setProjectedWaitingRoomState: deps.setWaitingRoomStateProjection,
    getSessions: deps.availableSessions,
    getProviderCatalog: deps.providerCatalogState,
    getRemoteState: () => ({
      ...managedWaitingRoomRemote(),
      cloudNotice: deps.waitingRoomCloudNotice(),
      collaborationBackend: relayCloudProfile(deps.preferencesState()) ? "cloud" : deps.relayStatusState()?.configured ? "relay" : "local",
      inventoryStatus: deps.waitingRoomInventoryStatus(),
      loadingFrame: deps.waitingRoomState().introStep,
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
      providerAccounts: deps.providerAccountsState(),
      terminals: deps.terminalsState(),
      slices: deps.slicesState(),
      externalProviderSessions: deps.externalProviderSessionsState(),
      externalProviderSessionsHasMore: deps.externalProviderSessionsPageState().hasMore,
      externalProviderSessionsNextCursor: deps.externalProviderSessionsPageState().nextCursor,
      projects: deps.waitingRoomProjects(),
    }),
    getThemeRegistry: deps.themeRegistryState,
    getCurrentProvider: () => deps.options.provider ?? "opencode",
    getCurrentModel: () => deps.options.model,
    setProviderDefaults: (defaults) => {
      deps.options.provider = defaults.provider
      deps.options.model = defaults.model
      deps.options.effort = defaults.effort
    },
    applyTheme,
    resetTranscriptSyntax: deps.resetTranscriptSyntax,
    bumpThemeRevision: () => {
      deps.setThemeRevision((revision: number) => revision + 1)
    },
    saveUiThemePreference: (themeId) => {
      void saveUiPreferences({ theme: themeId })
    },
    mergeUiThemePreference: (themeId) => {
      deps.setPreferencesState((current: any) => mergeUiPreferences(current, { theme: themeId }))
    },
    applyResponseLayout: () => deps.applyResponseLayout(),
    renderCommandCenter: () => deps.renderCommandCenter(),
    saveProviderPreferences: (provider, preferences) => {
      void saveProviderPreferences(provider, preferences)
    },
    isAttached: deps.isAttached,
    rebuildTranscript: () => deps.rebuildTranscript(),
    updateSessionChrome: () => deps.updateSessionChrome(),
    syncCommandCenter: () => deps.syncCommandCenter(),
    refreshProviderCatalogForSelection: (state) => {
      if (!deps.kernelConnected()) return
      const revision = ++providerCatalogSelectionRevision
      const executionLocation = state.sliceSelectionId && !["none", "new"].includes(state.sliceSelectionId)
        ? { kind: "slice" as const, slice_ref: state.sliceSelectionId }
        : state.selectedKernelRef && state.selectedKernelRef !== "local"
          ? { kind: "worker" as const, kernel_ref: state.selectedKernelRef }
          : { kind: "local" as const }
      void getProviderCatalog(deps.client, deps.appLogger, {
        provider: state.providerId,
        accountProfile: state.accountProfileId ?? "default",
        executionLocation,
      }, false).then((catalog) => {
        if (revision !== providerCatalogSelectionRevision) return
        deps.setProviderCatalogState(catalog)
        reconcileWaitingRoomProjection(deps.waitingRoomState())
      }).catch((error) => {
        deps.appLogger?.warn("failed to refresh provider catalog for account selection", {
          error: deps.formatError(error),
        })
      })
    },
  })
  const reconcileWaitingRoom = waitingRoomReconcileController.reconcile
  const reconcileWaitingRoomProjection = waitingRoomReconcileController.reconcileProjection

  const getInventoryForClient = (client: LocalIpcClient) => getWaitingRoomInventory(client, async () => {
    try { return await machineAllocationCapability(client) }
    catch (error) {
      deps.appLogger?.warn("failed to read machine allocation capability", { error: deps.formatError(error) })
      return undefined
    }
  })

  const waitingRoomInventoryRefreshController = createWaitingRoomInventoryRefreshController({
    isKernelConnected: deps.kernelConnected,
    getSessionPage: async (cursor, inventoryVersion) => {
      const response = await deps.client.currentClient().send({ ListWaitingRoomSessions: { cursor, limit: 500, inventory_version: inventoryVersion } }) as { WaitingRoomSessionsListed: { page: { sessions: import("./cli-types.js").WaitingRoomPublicSessionSummary[]; next_cursor: string | null } } }
      return response.WaitingRoomSessionsListed.page
    },
    getInventoryStatus: deps.waitingRoomInventoryStatus,
    setInventoryStatus: deps.setWaitingRoomInventoryStatus,
    getWaitingRoomState: deps.waitingRoomState,
    getInventory: async () => {
      const inventory = await waitingRoomWorkspaceController.readInventory(() => getInventoryForClient(deps.client.currentClient()))
      if (!homeKernelId) {
        homeKernelId = inventory.kernelId
        homeMachineId = inventory.machineId
      }
      return inventory
    },
    isKernelHidden: deps.waitingRoomHiddenKernelController.isKernelHidden,
    getAvailableSessions: deps.availableSessions,
    setAvailableSessions: deps.setAvailableSessions,
    getProjects: deps.waitingRoomProjects,
    setProjects: deps.setWaitingRoomProjects,
    setRelayStatus: deps.setRelayStatusState,
    setRemoteMachines: deps.setRemoteMachinesState,
    setRemoteKernels: deps.setRemoteKernelsState,
    setProviderAccountCopyOffers: offers => deps.setProviderAccountCopyOffersState?.(offers),
    setProviderAccounts: deps.setProviderAccountsState,
    setMachineAllocationCapability: (capability) => {
      allocationCapability = capability
    },
    getMachineAllocationCapabilityScope: (inventory) => {
      const profile = relayCloudProfile(deps.preferencesState())
      return JSON.stringify([
        inventory.kernelId,
        profile?.apiUrl ?? null,
        profile?.accountId ?? null,
        profile?.userId ?? null,
        profile?.realmId ?? null,
      ])
    },
    shouldApplyWorkspaceInventory: (inventory) => waitingRoomWorkspaceController.acceptsInventory(inventory)
      && (workspacePlacementController?.acceptsInventory(inventory.machineId, inventory.kernelId) ?? true),
    applyWorkspaceInventory: (inventory, client) => {
      return waitingRoomWorkspaceController.applyInventory(inventory, client ? request => client.send(request) : undefined)
        .then(() => reconcileWaitingRoomProjection(deps.waitingRoomState()))
        .catch((error) => deps.flashFooter(deps.formatError(error), "error"))
    },
    setLaunchTarget: (target) => {
      sourceLaunchTarget = target
    },
    setTerminals: deps.setTerminalsState,
    setSlices: deps.setSlicesState,
    setExternalProviderSessions: deps.setExternalProviderSessionsState,
    setExternalProviderSessionsPage: deps.setExternalProviderSessionsPageState,
    reconcileWaitingRoom: reconcileWaitingRoomProjection,
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
    cachedInventories: cachedWaitingRoomInventories,
    getCacheScopeKey: waitingRoomInventoryCacheScope,
    loadCachedInventories: waitingRoomInventoryCache.load,
    getDirectTargetKernelId: kernelConnectionController.getDirectTargetKernelId,
    persistInventory: waitingRoomInventoryCache.persist,
    getLocalKernelPresences: loadLocalKernelPresences,
  })
  const refreshCloudDirectory = createCloudWaitingRoomController({
    client: deps.cloudClient, isKernelConnected: deps.kernelConnected,
    setMachines: deps.setRemoteMachinesState, setKernels: deps.setRemoteKernelsState,
    setStatus: deps.setWaitingRoomInventoryStatus,
    reconcile: () => reconcileWaitingRoomProjection(deps.waitingRoomState()),
    captureActivity: () => {
      const generation = manualKernelSelectionGeneration
      const revision = deps.waitingRoomLaunchOwnershipRevision()
      return () => generation === manualKernelSelectionGeneration
        && revision === deps.waitingRoomLaunchOwnershipRevision() && !deps.isAttached()
    },
    preferredKernelRef: () => deps.waitingRoomState().selectedKernelRef,
    hydrateKernel: async (kernelId, machineId, isActive) => {
      if (!isActive()) return
      deps.setWaitingRoomStateProjection({ ...deps.waitingRoomState(), selectedKernelRef: kernelId, selectedMachineRef: machineId })
      await connectDetachedKernelFromWaitingRoom(isActive, true)
    },
  })
  const refreshWaitingRoomDataNow = async () => {
    if (!deps.kernelConnected()) return refreshCloudDirectory()
    await waitingRoomInventoryRefreshController.refreshNow()
  }
  const refreshWaitingRoomData = async () => {
    if (!deps.kernelConnected()) return refreshCloudDirectory()
    await waitingRoomInventoryRefreshController.refresh()
  }
  const applyWaitingRoomTransportClosed = waitingRoomInventoryRefreshController.applyTransportClosed
  const applyWaitingRoomRowsChanged = waitingRoomInventoryRefreshController.applyRowsChanged
  const applyRelayStatusChanged = waitingRoomInventoryRefreshController.applyRelayStatusChanged
  const applyRemoteMachinesChanged = waitingRoomInventoryRefreshController.applyRemoteMachinesChanged
  const applyProviderCatalogChanged = (catalog: ProviderCatalog) => {
    deps.setProviderCatalogState(catalog)
    reconcileWaitingRoomProjection(deps.waitingRoomState())
  }
  const applySlicesChanged = (slices: SliceRecord[]) => {
    deps.setSlicesState(slices)
    reconcileWaitingRoomProjection(deps.waitingRoomState())
  }

  const detachedKernelConnectController = createDetachedKernelConnectController({
    logInfo: (message, fields) => deps.appLogger?.info(message, fields),
    flashFooter: (message, tone) => deps.flashFooter(message, tone),
    getProviderCatalog: () => getProviderCatalog(deps.client, deps.appLogger),
    getProviderCommandCatalogs: () => getProviderCommandCatalogs(deps.client, deps.appLogger),
    invalidateWaitingRoomInventory: waitingRoomInventoryRefreshController.invalidate,
    setProviderCatalog: deps.setProviderCatalogState,
    setProviderCommandCatalogs: deps.setProviderCommandCatalogState,
    setKernelConnected: deps.setKernelConnected,
    setDaemonDisconnected: deps.setDaemonDisconnected,
    refreshWaitingRoomData,
  })
  const connectDetachedKernelFromWaitingRoom = async (isActive: () => boolean = () => true, automatic = false) => {
    if (!automatic) manualKernelSelectionGeneration++
    if (!isActive()) return
    const selected = deps.waitingRoomState()
    if (deps.cloudClient && !deps.kernelConnected() && await deps.cloudClient.profile()) {
      if (!selected.selectedKernelRef || selected.selectedKernelRef === "local") throw new Error("Choose an online kernel from My kernels first")
      if (!isActive()) return
      if (!await kernelConnectionController.connect(selected.selectedKernelRef, selected.selectedMachineRef, isActive)) return
    }
    const client = deps.client.currentClient()
    await detachedKernelConnectController.connect(() => isActive() && client === deps.client.currentClient())
  }

  const replaceClientForKernel: typeof kernelConnectionController.connect = (...args) => {
    manualKernelSelectionGeneration++
    return kernelConnectionController.connect(...args)
  }

  workspacePlacementController = createWaitingRoomWorkspacePlacementController({
    getState: deps.waitingRoomState,
    homeMachineId: () => homeMachineId,
    beginMachineSelection: waitingRoomWorkspaceController.beginMachineSelection,
    connect: replaceClientForKernel,
    homeKernelId: () => homeKernelId,
    refresh: refreshWaitingRoomDataNow,
    failure: (error) => deps.flashFooter(deps.formatError(error), "error"),
  })

  const allocationController = new WaitingRoomMachineAllocationController({
    acquire: (revision, requestId) => acquireMachineAllocation(deps.client, revision, requestId),
    get: allocationRef => getMachineAllocation(deps.client, allocationRef),
    requestId: randomUUID, now: Date.now, delay: ms => new Promise(resolve => setTimeout(resolve, ms)),
  })
  const acquireMachine = async (capability: MachineAllocationCapability) => {
    const sourceState = deps.waitingRoomState()
    const sourceRelay = deps.relayStatusState()
    const sourceWorkspace = deps.pendingWorkspaceTarget()
    const sourceWorktree = deps.pendingWorktreeTarget()
    let expectedMachine = sourceState.selectedMachineRef
    let expectedRevision = deps.waitingRoomLaunchOwnershipRevision()
    let pivot: MutableLocalIpcClientPivot | null = null
    const assertActive = () => {
      if (deps.isAttached() || deps.waitingRoomState().selectedMachineRef !== expectedMachine || deps.waitingRoomLaunchOwnershipRevision() !== expectedRevision) {
        throw new Error("Machine acquisition was cancelled because the Waiting Room selection changed. Retry to check the same allocation.")
      }
    }
    try {
      const machine = await allocationController.acquire(capability, { assertActive, allocated() {}, progress: message => deps.flashFooter(message, "info") })
      assertActive()
      const target = machine.kernel_target
      await approveRemoteMachine(deps.client.currentClient(), target.machine_id)
      assertActive()
      let inventory: WaitingRoomInventory | undefined
      const connected = await replaceClientForKernel(target.kernel_id, target.machine_id, () => {
        try { assertActive(); return true } catch { return false }
      }, next => {
        if (next.kernelId !== target.kernel_id || next.machineId !== target.machine_id) throw new Error("The connected kernel differs from the acquired machine.")
        inventory = next
      }, next => { pivot = next })
      assertActive()
      if (!connected || !inventory) throw new Error("The new machine is not reachable yet. Retry to check the same allocation.")
      expectedMachine = target.machine_id
      deps.setWaitingRoomStateProjection({ ...deps.waitingRoomState(), selectedMachineRef: target.machine_id, selectedKernelRef: target.kernel_id, projectSelectionId: "default", worktreeSelectionId: "", sliceSelectionId: "none" })
      waitingRoomWorkspaceController.beginMachineSelection(target.machine_id, target.kernel_id)
      expectedRevision = deps.waitingRoomLaunchOwnershipRevision()
      deps.setRelayStatusState(inventory.relayStatus)
      await waitingRoomWorkspaceController.applyInventory(inventory)
      assertActive()
      await refreshWaitingRoomDataNow()
      assertActive()
      await (pivot as MutableLocalIpcClientPivot | null)?.commit()
      allocationController.complete(machine.allocation_ref)
      deps.flashFooter("Machine ready.", "info")
    } catch (error) {
      await (pivot as MutableLocalIpcClientPivot | null)?.rollback()
      deps.setRelayStatusState(sourceRelay)
      if (deps.waitingRoomState().selectedMachineRef === expectedMachine && deps.waitingRoomLaunchOwnershipRevision() === expectedRevision) {
        deps.setWaitingRoomStateProjection(sourceState)
        deps.setPendingWorkspaceTarget(sourceWorkspace)
        deps.setPendingWorktreeTarget(sourceWorktree)
      }
      throw error
    }
  }

  const waitingRoomActivationController = createWaitingRoomActivationController({
    onLaunchRefused: (message, retry) => deps.onLaunchRefused?.(message, () => { if (!deps.isAttached()) retry() }),
    isKernelConnected: deps.kernelConnected,
    connectKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: deps.waitingRoomState,
    getRemoteState: () => ({
      ...managedWaitingRoomRemote(),
      inventoryStatus: deps.waitingRoomInventoryStatus(),
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
      providerAccounts: deps.providerAccountsState(),
      terminals: deps.terminalsState(),
      slices: deps.slicesState(),
      externalProviderSessions: deps.externalProviderSessionsState(),
      externalProviderSessionsHasMore: deps.externalProviderSessionsPageState().hasMore,
      externalProviderSessionsNextCursor: deps.externalProviderSessionsPageState().nextCursor,
      projects: deps.waitingRoomProjects(),
    }),
    getWorkspaceTarget: deps.pendingWorkspaceTarget,
    getWorktreeTarget: deps.pendingWorktreeTarget,
    getAvailableSessions: deps.availableSessions,
    getProviderCatalog: deps.providerCatalogState,
    getCurrentProvider: () => deps.options.provider ?? "opencode",
    getCurrentModel: () => deps.options.model,
    getAccountProfile: () => deps.waitingRoomState().accountProfileId,
    handleCloudCommand: () => deps.handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] }),
    setPromptText: deps.setPromptText,
    focusPrompt: deps.focusPrompt,
    syncCommandCenter: deps.syncCommandCenter,
    openTerminalPairingDialog: deps.openTerminalPairingDialog,
    openSessionBrowserDialog: deps.openSessionBrowserDialog,
    createSession: async (workspacePath, worktreePath, launch) => {
      return await createSession(deps.client, workspacePath, worktreePath, undefined, {
        provider: launch.provider,
        model: launch.model,
        effort: launch.effort,
        account_profile: launch.account_profile,
        execution_mode: launch.execution_mode,
        permission_level: launch.permission_level,
      }, launch.sliceRef, launch.workspaceLiveSyncMode, launch.sliceRef ? null : (launch.workerKernelRef ?? null), null, launch.projectSelection)
    },
    deleteCreatedSession: async (sessionId, workspacePath) => {
      await deleteSessionByRef(deps.client, sessionId, workspacePath)
    },
    importExternalProviderSession: (externalSessionId) => importExternalProviderSession(deps.client, externalSessionId),
    loadOlderExternalProviderSessions: async () => {
      const pageState = deps.externalProviderSessionsPageState()
      if (!pageState?.hasMore || !pageState.nextCursor) {
        return 0
      }
      const page = await listExternalProviderSessions(deps.client, { cursor: pageState.nextCursor })
      deps.setExternalProviderSessionsState((current: any[] = []) => {
        return mergeExternalProviderSessionsSorted(current, page.sessions)
      })
      deps.setExternalProviderSessionsPageState({
        hasMore: page.hasMore,
        nextCursor: page.nextCursor,
      })
      reconcileWaitingRoomProjection(deps.waitingRoomState())
      return page.sessions.length
    },
    browseKernelInventory: async (kernelId, machineId) => {
      await replaceClientForKernel(kernelId, machineId)
      await refreshWaitingRoomDataNow()
      return deps.availableSessions().filter((session: SessionListEntry) => {
        return (session.kernel_id ?? session.host_daemon_id) === kernelId
      }).length
    },
    createSlice: (options) => createSlice(deps.client, cliWaitingRoomSliceApiOptions(options)),
    startSlice: (sliceRef) => startSlice(deps.client, sliceRef),
    deleteSlice: (sliceRef) => deleteSlice(deps.client, sliceRef),
    updateSlices: (slice) => {
      deps.setSlicesState((current: any[] = []) => [
        slice,
        ...current.filter((candidate) => candidate.id !== slice.id),
      ])
    },
    prepareSessionOwnerClient: async (launch) => {
      await replaceClientForKernel(launch.ownerKernelRef, launch.ownerMachineRef)
      const inventory = await waitingRoomWorkspaceController.readInventory(() => getWaitingRoomInventory(deps.client.currentClient()))
      await waitingRoomWorkspaceController.applyInventory(inventory)
      // MP-08/MP-11: workspace previews retain home account state. Admission
      // after the owner pivot must use the destination's fresh inventory.
      deps.setProviderAccountCopyOffersState?.(inventory.providerAccountCopyOffers ?? [])
      deps.setProviderAccountsState(inventory.providerAccounts ?? [])
      // Profile IDs are kernel-scoped. Preserve a matching destination choice;
      // otherwise use its normal Waiting Room default before admission.
      launch.accountProfile = normalizeProviderAccountProfileId(
        inventory.providerAccounts, launch.provider, launch.accountProfile,
      )
      deps.setWaitingRoomStateProjection({ ...deps.waitingRoomState(), accountProfileId: launch.accountProfile })
    },
    acquireMachine,
    prepareExistingSessionClient: async (session) => {
      await replaceClientForKernel(session.kernel_id ?? session.host_daemon_id, session.machine_id ?? session.host_machine_id)
      await refreshWaitingRoomDataNow()
    },
    attachBinding: deps.attachBinding,
    rollbackAttachedSession: deps.rollbackAttachedSession,
    flashFooter: (message, tone) => deps.flashFooter(message, tone),
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
  })
  const activateWaitingRoom = waitingRoomActivationController.activate
  const startSessionFromWaitingRoomDefaults = waitingRoomActivationController.startSessionFromWaitingRoomDefaults
  const waitingRoomLifecycleConfirmationController = createWaitingRoomLifecycleConfirmationController()
  const waitingRoomLifecycleActionController = createWaitingRoomLifecycleActionController({
    isKernelConnected: deps.kernelConnected,
    connectDetachedKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: deps.waitingRoomState,
    getRemoteState: () => ({
      ...managedWaitingRoomRemote(),
      cloudNotice: deps.waitingRoomCloudNotice(),
      collaborationBackend: relayCloudProfile(deps.preferencesState()) ? "cloud" : deps.relayStatusState()?.configured ? "relay" : "local",
      inventoryStatus: deps.waitingRoomInventoryStatus(),
      loadingFrame: deps.waitingRoomState().introStep,
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
      providerAccounts: deps.providerAccountsState(),
      terminals: deps.terminalsState(),
      slices: deps.slicesState(),
      externalProviderSessions: deps.externalProviderSessionsState(),
      externalProviderSessionsHasMore: deps.externalProviderSessionsPageState().hasMore,
      externalProviderSessionsNextCursor: deps.externalProviderSessionsPageState().nextCursor,
      projects: deps.waitingRoomProjects(),
    }),
    getAvailableSessions: deps.availableSessions,
    setAvailableSessions: deps.setAvailableSessions,
    getProviderCatalog: deps.providerCatalogState,
    getProjects: deps.waitingRoomProjects,
    archiveProject: (projectId) => archiveProject(deps.client, projectId),
    deleteProject: (projectId) => deleteProject(deps.client, projectId),
    restoreProject: (projectId) => restoreProject(deps.client, projectId),
    renameProject: (projectId, name) => renameProject(deps.client, projectId, name),
    getWorkspaceTarget: deps.pendingWorkspaceTarget,
    confirmationController: waitingRoomLifecycleConfirmationController,
    archiveSessionById: (sessionId) => archiveSessionById(deps.client, sessionId),
    deleteSessionByRef: (sessionRef, workspace) => deleteSessionByRef(deps.client, sessionRef, workspace),
    forgetRemoteMachine: (machineRef) => forgetRemoteMachine(deps.client, machineRef),
    getRemoteMachines: deps.remoteMachinesState,
    setRemoteMachines: deps.setRemoteMachinesState,
    getRemoteKernels: deps.remoteKernelsState,
    setRemoteKernels: deps.setRemoteKernelsState,
    getSlices: deps.slicesState,
    setSlices: deps.setSlicesState,
    deleteSlice: (sliceRef) => deleteSlice(deps.client, sliceRef),
    hideRemoteKernel: deps.waitingRoomHiddenKernelController.hideKernel,
    invalidateInventory: waitingRoomInventoryRefreshController.invalidate,
    reconcileWaitingRoom,
    refreshWaitingRoomData: () => refreshWaitingRoomData(),
    sessionBrowserOpen: deps.sessionBrowserOpen,
    closeSessionBrowserDialog: deps.closeSessionBrowserDialog,
    flashFooter: (message, tone) => deps.flashFooter(message, tone),
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
  })
  const applyWaitingRoomSessionLifecycleAction = waitingRoomLifecycleActionController.applyAction
  const restoreWaitingRoomProject = waitingRoomLifecycleActionController.restoreProject
  const renameWaitingRoomProject = waitingRoomLifecycleActionController.renameProject

  const providerPromptProjectionController = createProviderPromptProjectionController({
    getProviderRun: deps.focusedProviderRun,
    getFocusedAgent: deps.focusedAgent,
    getWaitingRoomState: deps.waitingRoomState,
    getDefaults: () => ({
      provider: deps.options.provider ?? "opencode",
      accountProfile: deps.options.accountProfile ?? "default",
      model: deps.options.model,
      effort: deps.options.effort,
    }),
    getProviderCatalog: deps.providerCatalogState,
  })
  const currentProviderSelection = providerPromptProjectionController.currentProviderSelection

  const providerSelectionController = createProviderSelectionController({
    currentProviderSelection,
    waitingRoomState: deps.waitingRoomState,
    availableSessions: deps.availableSessions,
    providerCatalog: deps.providerCatalogState,
    providerAccounts: deps.providerAccountsState,
    loadProviderCatalog: async (provider, accountProfile) => {
      const agent = deps.focusedAgent()
      const sliceRef = agent?.runtime_placement?.slice_id?.trim()
      const workerRef = agent?.remote_execution?.worker_kernel_id?.trim()
        || (!sliceRef && agent?.remote_execution ? agent.runtime_placement?.kernel_id?.trim() : "")
      return getProviderCatalog(deps.client, deps.appLogger, {
        provider,
        accountProfile,
        executionLocation: sliceRef
          ? { kind: "slice", slice_ref: sliceRef }
          : workerRef
            ? { kind: "worker", kernel_ref: workerRef }
            : { kind: "local" },
      }, false)
    },
    setProviderCatalog: deps.setProviderCatalogState,
    themeRegistry: deps.themeRegistryState,
    preferences: deps.preferencesState,
    defaults: () => ({
      provider: deps.options.provider ?? "opencode",
      accountProfile: deps.options.accountProfile ?? "default",
      model: deps.options.model,
      effort: deps.options.effort,
    }),
    setDefaults: (selection) => {
      deps.options.provider = selection.provider
      deps.options.accountProfile = selection.accountProfile ?? deps.options.accountProfile ?? "default"
      deps.options.model = selection.model
      deps.options.effort = selection.effort
    },
    reconcileWaitingRoom,
    isAttached: deps.isAttached,
    focusedAgentId: deps.focusedAgentId,
    providerRunState: deps.providerRunState,
    sessionState: deps.sessionState,
    updateAgentProfile: (sessionId, agentId, profile) => updateAgentProfile(deps.client, sessionId, agentId, profile),
    updateAgentConfig: (sessionId, agentId, config) => updateAgentConfig(deps.client, sessionId, agentId, config),
    applySessionState: deps.applySessionState,
    clearProviderRunState: () => deps.setProviderRunState(null),
    getProviderAuthStatus: (provider, accountProfile) => getProviderAuthStatus(deps.client, provider, accountProfile),
    appendNotice: deps.appendNotice,
    flashFooter: (message, tone) => deps.flashFooter(message, tone),
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
  })

  const waitingRoomTargets = () => ({
    workspacePath: deps.pendingWorkspaceTarget(),
    worktreePath: deps.pendingWorktreeTarget(),
    workspaceId: sourceLaunchTarget?.workspaceId,
    worktreeId: sourceLaunchTarget?.worktreeId,
    ...waitingRoomAllocationRemoteState(allocationCapability),
  })

  return {
    activateWaitingRoom,
    applyAccountSelection: providerSelectionController.applyAccountSelection,
    applyModelSelection: providerSelectionController.applyModelSelection,
    applyModeSelection: providerSelectionController.applyModeSelection,
    applyPermissionSelection: providerSelectionController.applyPermissionSelection,
    applyProviderSelection: providerSelectionController.applyProviderSelection,
    applyVariantSelection: providerSelectionController.applyVariantSelection,
    applyProviderCatalogChanged,
    applyRelayStatusChanged,
    applyRemoteMachinesChanged,
    applySlicesChanged,
    applyWaitingRoomSessionLifecycleAction,
    restoreWaitingRoomProject,
    renameWaitingRoomProject,
    applyWaitingRoomTransportClosed,
    applyWaitingRoomRowsChanged,
    connectDetachedKernelFromWaitingRoom,
    currentModelId: providerPromptProjectionController.currentModelId,
    currentProviderSelection,
    currentVariantId: providerPromptProjectionController.currentVariantId,
    promptMetaParts: providerPromptProjectionController.promptMetaParts,
    promptUsageMeta: providerPromptProjectionController.promptUsageMeta,
    reconcileWaitingRoom,
    reconcileWaitingRoomProjection,
    refreshWaitingRoomData,
    loadWaitingRoomSessionHistory: waitingRoomInventoryRefreshController.loadSessionHistory,
    refreshWaitingRoomDataNow,
    editWaitingRoomWorkspace: async (path: string) => {
      await waitingRoomWorkspaceController.editWorkspace(path)
      reconcileWaitingRoomProjection(deps.waitingRoomState())
    },
    startSessionFromWaitingRoomDefaults,
    waitingRoomTargets,
  }
}
