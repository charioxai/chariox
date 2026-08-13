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
import { createWaitingRoomActivationController } from "./waiting-room-activation-controller.js"
import { getWaitingRoomInventory } from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import { createWaitingRoomInventoryCache } from "./waiting-room-inventory-cache.js"
import { createWaitingRoomLifecycleActionController } from "./waiting-room-lifecycle-action-controller.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import { createWaitingRoomReconcileController } from "./waiting-room-reconcile-controller.js"
import {
  archiveProject,
  deleteProject,
  renameProject,
  restoreProject,
} from "./project-api.js"

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
  terminalsState: AnyFn
  setTerminalsState: AnyFn
  slicesState: AnyFn
  setSlicesState: AnyFn
  externalProviderSessionsState: AnyFn
  setExternalProviderSessionsState: AnyFn
  externalProviderSessionsPageState: AnyFn
  setExternalProviderSessionsPageState: AnyFn
  pendingWorkspaceTarget: AnyFn
  pendingWorktreeTarget: AnyFn
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
  const waitingRoomInventoryCache = createWaitingRoomInventoryCache()
  const cachedWaitingRoomInventories = waitingRoomInventoryCache.load()
  const waitingRoomReconcileController = createWaitingRoomReconcileController({
    getCurrentState: deps.waitingRoomState,
    setWaitingRoomState: deps.setWaitingRoomState,
    getSessions: deps.availableSessions,
    getProviderCatalog: deps.providerCatalogState,
    getRemoteState: () => ({
      workspaceId: deps.pendingWorkspaceTarget(),
      cloudNotice: deps.waitingRoomCloudNotice(),
      collaborationBackend: relayCloudProfile(deps.preferencesState()) ? "cloud" : deps.relayStatusState()?.configured ? "relay" : "local",
      inventoryStatus: deps.waitingRoomInventoryStatus(),
      loadingFrame: deps.waitingRoomState().introStep,
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
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
  })
  const reconcileWaitingRoom = waitingRoomReconcileController.reconcile

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
    setTerminals: deps.setTerminalsState,
    setSlices: deps.setSlicesState,
    setExternalProviderSessions: deps.setExternalProviderSessionsState,
    setExternalProviderSessionsPage: deps.setExternalProviderSessionsPageState,
    reconcileWaitingRoom,
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
    cachedInventories: cachedWaitingRoomInventories,
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
    reconcileWaitingRoom(deps.waitingRoomState())
  }
  const applySlicesChanged = (slices: SliceRecord[]) => {
    deps.setSlicesState(slices)
    reconcileWaitingRoom(deps.waitingRoomState())
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

  const replaceClientForKernel = async (kernelRef: string | null | undefined, machineRef: string | null | undefined) => {
    const targetKernelRef = kernelRef?.trim()
    const currentKernelId = deps.relayStatusState()?.daemon_id?.trim()
    if (!targetKernelRef || targetKernelRef === "local" || targetKernelRef === currentKernelId) {
      return
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
    try {
      await getWaitingRoomInventory(nextClient)
    } catch (error) {
      await nextClient.close()
      throw error
    }
    await deps.client.replaceClient(nextClient)
    waitingRoomInventoryRefreshController.invalidate()
    deps.setKernelConnected(true)
    deps.setDaemonDisconnected(false)
    deps.flashFooter(`connected to kernel ${localPresence?.kernelAlias ?? connection?.targetDaemonAlias ?? connection?.kernelId ?? targetKernelRef}`, "info")
  }

  const waitingRoomActivationController = createWaitingRoomActivationController({
    isKernelConnected: deps.kernelConnected,
    connectKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: deps.waitingRoomState,
    getRemoteState: () => ({
      workspaceId: deps.pendingWorkspaceTarget(),
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
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
    getAccountProfile: () => deps.options.accountProfile,
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
      reconcileWaitingRoom(deps.waitingRoomState())
      return page.sessions.length
    },
    browseKernelInventory: async (kernelId, machineId) => {
      await replaceClientForKernel(kernelId, machineId)
      await refreshWaitingRoomDataNow()
      return deps.availableSessions().filter((session: SessionListEntry) => {
        return (session.kernel_id ?? session.host_daemon_id) === kernelId
      }).length
    },
    createSlice: (options) => createSlice(deps.client, {
      name: options.name,
      displayMode: options.displayMode,
      workspaceId: options.workspaceId,
      worktreeId: options.worktreeId,
      workspaceMount: options.workspaceMount,
      ...(options.workerKernelRef !== undefined ? { workerKernelRef: options.workerKernelRef } : {}),
    }),
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
    prepareExistingSessionClient: async (session) => {
      await replaceClientForKernel(session.kernel_id ?? session.host_daemon_id, session.machine_id ?? session.host_machine_id)
      await refreshWaitingRoomDataNow()
    },
    attachBinding: deps.attachBinding,
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
      workspaceId: deps.pendingWorkspaceTarget(),
      cloudNotice: deps.waitingRoomCloudNotice(),
      collaborationBackend: relayCloudProfile(deps.preferencesState()) ? "cloud" : deps.relayStatusState()?.configured ? "relay" : "local",
      inventoryStatus: deps.waitingRoomInventoryStatus(),
      loadingFrame: deps.waitingRoomState().introStep,
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
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
    themeRegistry: deps.themeRegistryState,
    preferences: deps.preferencesState,
    defaults: () => ({
      provider: deps.options.provider ?? "opencode",
      model: deps.options.model,
      effort: deps.options.effort,
    }),
    setDefaults: (selection) => {
      deps.options.provider = selection.provider
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
    getProviderAuthStatus: (provider) => getProviderAuthStatus(deps.client, provider),
    appendNotice: deps.appendNotice,
    flashFooter: (message, tone) => deps.flashFooter(message, tone),
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
    formatError: deps.formatError,
  })

  const waitingRoomTargets = () => ({
    workspacePath: deps.pendingWorkspaceTarget(),
    worktreePath: deps.pendingWorktreeTarget(),
  })

  return {
    activateWaitingRoom,
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
    refreshWaitingRoomData,
    refreshWaitingRoomDataNow,
    startSessionFromWaitingRoomDefaults,
    waitingRoomTargets,
  }
}
