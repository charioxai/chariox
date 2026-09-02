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
import { LocalIpcClient } from "./ipc.js"
import { loadLocalKernelPresences, localKernelEndpoint } from "./local-kernel-presence.js"
import {
  getProviderAuthStatus,
  getProviderCatalog,
  getProviderCommandCatalogs,
} from "./provider-api.js"
import { createProviderPromptProjectionController } from "./provider-prompt-projection-controller.js"
import { createProviderSelectionController } from "./provider-selection-controller.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { forgetRemoteMachine } from "./remote-machine-api.js"
import { resolveKernelClientConnection } from "./relay-api.js"
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
  type WaitingRoomPreparedManagedLaunch,
} from "./waiting-room-activation-controller.js"
import { cliWaitingRoomSliceApiOptions } from "./waiting-room-slice-api-options.js"
import type { WaitingRoomLaunchConfig } from "./waiting-room-controller.js"
import {
  createManagedEnvironment,
  getManagedContextLaunchTarget,
  getManagedContextTransferStatus,
  getManagedEnvironment,
  prepareManagedEnvironmentContextTransfer,
  requestManagedEnvironmentLifecycle,
  startManagedContextTransfer,
} from "./managed-environment-api.js"
import {
  WaitingRoomManagedEnvironmentLaunchController,
} from "./waiting-room-managed-environment-launch-controller.js"
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
import type {
  ManagedEnvironmentCatalog,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import {
  clearStagedWaitingRoomWorktreeSelection,
} from "./waiting-room-worktrees.js"
import { existingProjectSelectionId } from "./waiting-room-projects.js"
import {
  managedEnvironmentMachineRef,
} from "./waiting-room-managed-environments.js"
import {
  beginMutableLocalIpcClientPivot,
  type MutableLocalIpcClientPivot,
} from "./mutable-local-ipc-client.js"

type AnyFn = (...args: any[]) => any

export type CliWaitingRoomCompositionDeps = {
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
  let directTargetKernelId = deps.options.targetDaemonId?.trim() || null
  let providerCatalogSelectionRevision = 0
  let managedEnvironmentCatalog: ManagedEnvironmentCatalog | undefined
  let sourceLaunchTarget: { workspaceId: string; worktreeId: string } | null | undefined
  const managedWaitingRoomRemote = () => ({
    ...(sourceLaunchTarget
      ? { workspaceId: sourceLaunchTarget.workspaceId, worktreeId: sourceLaunchTarget.worktreeId }
      : {}),
    ...(managedEnvironmentCatalog
      ? {
          managedComputeClasses: managedEnvironmentCatalog.computeClasses,
          managedContextSources: managedEnvironmentCatalog.contextSources,
          managedEnvironments: managedEnvironmentCatalog.environments,
        }
      : {}),
  })
  const waitingRoomReconcileController = createWaitingRoomReconcileController({
    getCurrentState: deps.waitingRoomState,
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

  const waitingRoomInventoryRefreshController = createWaitingRoomInventoryRefreshController({
    isKernelConnected: deps.kernelConnected,
    getInventoryStatus: deps.waitingRoomInventoryStatus,
    setInventoryStatus: deps.setWaitingRoomInventoryStatus,
    getWaitingRoomState: deps.waitingRoomState,
    getInventory: () => getWaitingRoomInventory(deps.client),
    isKernelHidden: deps.waitingRoomHiddenKernelController.isKernelHidden,
    getAvailableSessions: deps.availableSessions,
    setAvailableSessions: deps.setAvailableSessions,
    getProjects: deps.waitingRoomProjects,
    setProjects: deps.setWaitingRoomProjects,
    setRelayStatus: deps.setRelayStatusState,
    setRemoteMachines: deps.setRemoteMachinesState,
    setRemoteKernels: deps.setRemoteKernelsState,
    setProviderAccounts: deps.setProviderAccountsState,
    setManagedEnvironmentCatalog: (catalog) => {
      managedEnvironmentCatalog = catalog
    },
    getManagedEnvironmentCatalogScope: (inventory) => {
      const profile = relayCloudProfile(deps.preferencesState())
      return JSON.stringify([
        inventory.kernelId,
        profile?.apiUrl ?? null,
        profile?.accountId ?? null,
        profile?.userId ?? null,
        profile?.realmId ?? null,
      ])
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
    getDirectTargetKernelId: () => directTargetKernelId,
    persistInventory: waitingRoomInventoryCache.persist,
    getLocalKernelPresences: loadLocalKernelPresences,
  })
  const refreshWaitingRoomDataNow = waitingRoomInventoryRefreshController.refreshNow
  const refreshWaitingRoomData = waitingRoomInventoryRefreshController.refresh
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
  const connectDetachedKernelFromWaitingRoom = detachedKernelConnectController.connect

  const replaceClientForKernel = async (
    kernelRef: string | null | undefined,
    machineRef: string | null | undefined,
    isActive: () => boolean = () => true,
    connected?: (inventory: WaitingRoomInventory) => void,
    retainPrevious?: (pivot: MutableLocalIpcClientPivot) => void,
  ): Promise<boolean> => {
    const targetKernelRef = kernelRef?.trim()
    const currentKernelId = deps.relayStatusState()?.daemon_id?.trim()
    const sourceTargetKernelId = directTargetKernelId
    if (!targetKernelRef || targetKernelRef === "local" || targetKernelRef === currentKernelId) {
      if (connected && targetKernelRef && targetKernelRef !== "local") {
        const inventory = await getWaitingRoomInventory(deps.client)
        if (!isActive()) return false
        connected(inventory)
      }
      return isActive()
    }
    const localPresence = loadLocalKernelPresences()
      .find((presence) => presence.kernelId === targetKernelRef)
    const connection = localPresence
      ? null
      : await resolveKernelClientConnection(deps.client, {
          kernelRef: targetKernelRef,
          machineRef: machineRef ?? null,
          clientId: deps.options.clientId,
        })
    if (!isActive()) {
      return false
    }
    const nextClient = localPresence
      ? new LocalIpcClient(localKernelEndpoint(localPresence))
      : new LocalIpcClient(connection!.relayUrl, {
          relayAuthToken: connection!.relayToken,
          targetDaemonId: connection!.targetDaemonId ?? undefined,
          targetDaemonAlias: connection!.targetDaemonAlias ?? undefined,
        })
    if (typeof deps.client.replaceClient !== "function") {
      await nextClient.close()
      throw new Error("kernel client pivot is unavailable in this build")
    }
    let targetInventory: WaitingRoomInventory
    try {
      targetInventory = await getWaitingRoomInventory(nextClient)
    } catch (error) {
      await nextClient.close()
      throw error
    }
    if (!isActive()) {
      await nextClient.close()
      return false
    }
    if (retainPrevious) {
      if (typeof deps.client.swapClient !== "function") {
        await nextClient.close()
        throw new Error("transactional kernel client pivot is unavailable in this build")
      }
      const pivot = beginMutableLocalIpcClientPivot(deps.client, nextClient)
      try {
        directTargetKernelId = targetInventory.kernelId
        connected?.(targetInventory)
        retainPrevious({
          commit: () => pivot.commit(),
          rollback: async () => {
            try {
              await pivot.rollback()
            } finally {
              directTargetKernelId = sourceTargetKernelId
              waitingRoomInventoryRefreshController.invalidate()
            }
          },
        })
      } catch (error) {
        try {
          await pivot.rollback()
        } finally {
          directTargetKernelId = sourceTargetKernelId
        }
        throw error
      }
    } else {
      await deps.client.replaceClient(nextClient)
      directTargetKernelId = targetInventory.kernelId
      connected?.(targetInventory)
    }
    waitingRoomInventoryRefreshController.invalidate()
    deps.setKernelConnected(true)
    deps.setDaemonDisconnected(false)
    deps.flashFooter(`connected to kernel ${localPresence?.kernelAlias ?? connection?.targetDaemonAlias ?? connection?.kernelId ?? targetKernelRef}`, "info")
    return isActive()
  }

  const managedEnvironmentLaunchController = new WaitingRoomManagedEnvironmentLaunchController({
    createEnvironment: (input) => createManagedEnvironment(deps.client, input),
    getEnvironment: (environmentId) => getManagedEnvironment(deps.client, environmentId),
    requestLifecycle: (input) => requestManagedEnvironmentLifecycle(deps.client, input),
    prepareContextTransfer: (environmentId) => prepareManagedEnvironmentContextTransfer(
      deps.client,
      environmentId,
    ),
    startContextTransfer: (ticket) => startManagedContextTransfer(deps.client, ticket),
    getContextTransferStatus: (contextId) => getManagedContextTransferStatus(deps.client, contextId),
    connectKernel: async (machineId, kernelId, isActive) => {
      const sourceRelay = deps.relayStatusState()
      const connectedTarget: { inventory: WaitingRoomInventory | null } = { inventory: null }
      let clientPivot: MutableLocalIpcClientPivot | null = null
      if (!await replaceClientForKernel(kernelId, machineId, isActive, (inventory) => {
        const relay = inventory.relayStatus
        if (relay.daemon_id !== kernelId || relay.machine_id !== machineId) {
          throw new Error("connected kernel identity does not match the managed environment binding")
        }
        connectedTarget.inventory = inventory
        deps.setRelayStatusState(relay)
      }, (pivot) => {
        clientPivot = pivot
      })) {
        return null
      }
      const relay = connectedTarget.inventory?.relayStatus ?? deps.relayStatusState()
      if (relay?.daemon_id !== kernelId || relay.machine_id !== machineId) {
        throw new Error("connected kernel identity does not match the managed environment binding")
      }
      deps.setRelayStatusState(relay)
      let settled = false
      const commit = async () => {
        if (settled) return
        settled = true
        const pivot = clientPivot
        clientPivot = null
        try {
          await pivot?.commit()
        } catch (error) {
          deps.appLogger?.warn("failed to close the previous kernel client after managed launch", {
            error: deps.formatError(error),
          })
        }
      }
      const rollback = async () => {
        if (settled) return
        settled = true
        const pivot = clientPivot
        clientPivot = null
        try {
          await pivot?.rollback()
        } finally {
          if (pivot) deps.setRelayStatusState(sourceRelay)
        }
      }
      if (relay.connected !== true) {
        await rollback()
        return null
      }
      return { commit, rollback }
    },
    getLaunchTarget: (contextId, planDigest) => getManagedContextLaunchTarget(
      deps.client,
      contextId,
      planDigest,
    ),
    createIdempotencyKey: randomUUID,
    environmentName: () => "Managed agent",
    delay: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    nowMs: Date.now,
  })

  const prepareManagedSessionLaunch = async (
    launch: WaitingRoomLaunchConfig,
  ): Promise<WaitingRoomPreparedManagedLaunch> => {
    const selection = launch.managedEnvironment
    if (!selection) {
      throw new Error("managed session launch selection is missing")
    }
    let expectedMachineRef = deps.waitingRoomState().selectedMachineRef
    let expectedOwnershipRevision = deps.waitingRoomLaunchOwnershipRevision()
    let cancelled = false
    const assertActive = () => {
      if (cancelled
        || deps.waitingRoomState().selectedMachineRef !== expectedMachineRef
        || deps.waitingRoomLaunchOwnershipRevision() !== expectedOwnershipRevision) {
        cancelled = true
        throw new Error("managed session launch was cancelled because the Waiting Room selection changed")
      }
    }
    const environmentChanged = (environment: ManagedEnvironmentSummary) => {
      assertActive()
      if (managedEnvironmentCatalog) {
        managedEnvironmentCatalog = {
          ...managedEnvironmentCatalog,
          environments: [
            environment,
            ...managedEnvironmentCatalog.environments.filter((candidate) => (
              candidate.environmentId !== environment.environmentId
            )),
          ],
        }
      }
      expectedMachineRef = managedEnvironmentMachineRef(environment.environmentId)
      deps.setWaitingRoomState({
        ...deps.waitingRoomState(),
        selectedMachineRef: expectedMachineRef,
        ...(environment.runtimeKernelId ? { selectedKernelRef: environment.runtimeKernelId } : {}),
      })
      expectedOwnershipRevision = deps.waitingRoomLaunchOwnershipRevision()
      deps.rebuildTranscript()
    }
    const prepared = await managedEnvironmentLaunchController.prepare(selection, {
      assertActive,
      environmentChanged,
      progress: (message) => deps.flashFooter(message, "info"),
    })
    try {
      assertActive()
      deps.setPendingWorkspaceTarget(prepared.workspacePath)
      deps.setPendingWorktreeTarget(prepared.worktreePath)
      clearStagedWaitingRoomWorktreeSelection()
      const {
        managedEnvironment: _managedEnvironment,
        workerKernelRef: _workerKernelRef,
        sliceRef: _sliceRef,
        sliceCreate: _sliceCreate,
        projectSelection: _projectSelection,
        ...ordinaryLaunch
      } = launch
      expectedMachineRef = managedEnvironmentMachineRef(prepared.environment.environmentId)
      deps.setWaitingRoomState({
        ...deps.waitingRoomState(),
        selectedMachineRef: expectedMachineRef,
        selectedKernelRef: prepared.environment.runtimeKernelId ?? "",
        projectSelectionId: prepared.projectSelection.kind === "existing"
          ? existingProjectSelectionId(prepared.projectSelection.project_id)
          : "default",
        worktreeSelectionId: `existing:${prepared.worktreePath}`,
        sliceSelectionId: "none",
      })
      expectedOwnershipRevision = deps.waitingRoomLaunchOwnershipRevision()
      deps.rebuildTranscript()
      return {
        launch: {
          ...ordinaryLaunch,
          ownerMachineRef: prepared.environment.runtimeMachineId,
          ownerKernelRef: prepared.environment.runtimeKernelId,
          projectSelection: prepared.projectSelection,
        },
        assertActive,
        commit: prepared.commit,
        rollback: prepared.rollback,
      }
    } catch (error) {
      try {
        await prepared.rollback()
      } catch (rollbackError) {
        throw new Error(
          `${deps.formatError(error)}; failed to restore the source kernel connection: ${deps.formatError(rollbackError)}`,
        )
      }
      throw error
    }
  }

  const waitingRoomActivationController = createWaitingRoomActivationController({
    isKernelConnected: deps.kernelConnected,
    connectKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: deps.waitingRoomState,
    getRemoteState: () => ({
      ...managedWaitingRoomRemote(),
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
    createSession: (workspacePath, worktreePath, launch) => createSession(deps.client, workspacePath, worktreePath, undefined, {
      provider: launch.provider,
      model: launch.model,
      effort: launch.effort,
      account_profile: launch.account_profile,
      execution_mode: launch.execution_mode,
      permission_level: launch.permission_level,
    }, launch.sliceRef, launch.workspaceLiveSyncMode, launch.sliceRef ? null : (launch.workerKernelRef ?? null), null, launch.projectSelection),
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
    updateSlices: (slice) => {
      deps.setSlicesState((current: any[] = []) => [
        slice,
        ...current.filter((candidate) => candidate.id !== slice.id),
      ])
    },
    prepareSessionOwnerClient: async (launch) => {
      await replaceClientForKernel(launch.ownerKernelRef, launch.ownerMachineRef)
    },
    prepareManagedSessionLaunch,
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
    managedEnvironmentCatalog,
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
    refreshWaitingRoomDataNow,
    startSessionFromWaitingRoomDefaults,
    waitingRoomTargets,
  }
}
