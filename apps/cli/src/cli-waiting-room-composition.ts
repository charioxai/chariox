import { updateAgentProfile } from "./agent-api.js"
import { createDetachedKernelConnectController } from "./detached-kernel-connect-controller.js"
import { importExternalProviderSession, listExternalProviderSessions } from "./external-provider-session-api.js"
import type { SliceRecord } from "./cli-types.js"
import {
  saveProviderPreferences,
  saveUiPreferences,
  mergeUiPreferences,
  relayCloudProfile,
} from "./preferences.js"
import {
  getProviderAuthStatus,
  getProviderCatalog,
  getProviderCommandCatalogs,
} from "./provider-api.js"
import { createProviderPromptProjectionController } from "./provider-prompt-projection-controller.js"
import { createProviderSelectionController } from "./provider-selection-controller.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { forgetRemoteMachine } from "./remote-machine-api.js"
import {
  archiveSessionById,
  createSession,
  deleteSessionByRef,
} from "./session-api.js"
import {
  createSlice,
  deleteSlice,
  startSlice,
} from "./slice-api.js"
import { applyTheme } from "./theme.js"
import { createWaitingRoomActivationController } from "./waiting-room-activation-controller.js"
import { getWaitingRoomInventory } from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import { createWaitingRoomLifecycleActionController } from "./waiting-room-lifecycle-action-controller.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import { createWaitingRoomReconcileController } from "./waiting-room-reconcile-controller.js"

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
  const waitingRoomReconcileController = createWaitingRoomReconcileController({
    getCurrentState: deps.waitingRoomState,
    setWaitingRoomState: deps.setWaitingRoomState,
    getSessions: deps.availableSessions,
    getProviderCatalog: deps.providerCatalogState,
    getRemoteState: () => ({
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

  const waitingRoomActivationController = createWaitingRoomActivationController({
    isKernelConnected: deps.kernelConnected,
    connectKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: deps.waitingRoomState,
    getRemoteState: () => ({
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
      terminals: deps.terminalsState(),
      slices: deps.slicesState(),
      externalProviderSessions: deps.externalProviderSessionsState(),
      externalProviderSessionsHasMore: deps.externalProviderSessionsPageState().hasMore,
      externalProviderSessionsNextCursor: deps.externalProviderSessionsPageState().nextCursor,
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
    }, launch.sliceRef, launch.workspaceLiveSyncMode, launch.sliceRef ? null : launch.kernelRef, launch.metaagent === true),
    importExternalProviderSession: (externalSessionId) => importExternalProviderSession(deps.client, externalSessionId),
    loadOlderExternalProviderSessions: async () => {
      const pageState = deps.externalProviderSessionsPageState()
      if (!pageState?.hasMore || !pageState.nextCursor) {
        return 0
      }
      const page = await listExternalProviderSessions(deps.client, { cursor: pageState.nextCursor })
      deps.setExternalProviderSessionsState((current: any[] = []) => {
        const seen = new Set(current.map((session) => session.external_session_id))
        return [
          ...current,
          ...page.sessions.filter((session) => !seen.has(session.external_session_id)),
        ]
      })
      deps.setExternalProviderSessionsPageState({
        hasMore: page.hasMore,
        nextCursor: page.nextCursor,
      })
      reconcileWaitingRoom(deps.waitingRoomState())
      return page.sessions.length
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
    }),
    getAvailableSessions: deps.availableSessions,
    setAvailableSessions: deps.setAvailableSessions,
    getProviderCatalog: deps.providerCatalogState,
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
    applyProviderSelection: providerSelectionController.applyProviderSelection,
    applyVariantSelection: providerSelectionController.applyVariantSelection,
    applyProviderCatalogChanged,
    applyRelayStatusChanged,
    applyRemoteMachinesChanged,
    applySlicesChanged,
    applyWaitingRoomSessionLifecycleAction,
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
