import process from "node:process"

import { createCliExitController } from "./cli-exit-controller.js"
import { createKernelEventSubscriptionController } from "./kernel-event-subscription-controller.js"
import { createKernelRestartRecoveryController } from "./kernel-restart-recovery-controller.js"
import { createProviderRecoveryController } from "./provider-recovery-controller.js"
import { createSessionAttachmentController } from "./session-attachment-controller.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import {
  archiveSessionById,
  attachToSession,
  detachSessionAttachment,
  getSessionState,
  listSessions,
} from "./session-api.js"
import {
  getProviderCatalog,
  launchProviderRun,
  tryGetProviderRun,
} from "./provider-api.js"
import { getTerminalCommandCatalog } from "./terminal-command-catalog-api.js"
import {
  DEFAULT_CONNECTED_STATUS,
  getExitCleanupDecision,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
import {
  applyProviderRunProfileToSession,
} from "@arroba/kernel-client/prompt-provider-selection"
import {
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
} from "./session-state.js"
import { normalizeBackendProviderId } from "./provider-catalog.js"
import { createTerminalExitController } from "./terminal-exit-controller.js"
import { createWaitingRoomTransitionController } from "./waiting-room-transition-controller.js"

type AnyFn = (...args: any[]) => any

export type CliSessionLifecycleCompositionDeps = {
  client: any
  options: any
  appLogger: any
  renderer: any
  sleep: AnyFn
  formatError: AnyFn
  supportsKernelEventStream: boolean
  closingStateController: {
    isClosing: AnyFn
    setClosing: AnyFn
  }
  isAttached: AnyFn
  daemonDisconnected: AnyFn
  attachmentState: AnyFn
  sessionState: AnyFn
  providerRunState: AnyFn
  createdSessionState: AnyFn
  waitingRoomState: AnyFn
  preferencesState: AnyFn
  connectedClientCount: AnyFn
  persistablePromptDraft: AnyFn
  syncPromptTextSnapshot: AnyFn
  flushPendingPromptDraftPersist: AnyFn
  persistSessionPromptState: AnyFn
  applySessionState: AnyFn
  refreshAgentPanes: AnyFn
  refreshSplitPaneFocusRepaint: AnyFn
  maybeResize: AnyFn
  catchUpAttachedSession: AnyFn
  primeAttachedSessionBinding: AnyFn
  clearLocalBusyStateForAuthoritativeIdle: AnyFn
  recordDaemonActivity: AnyFn
  currentModelId: AnyFn
  currentVariantId: AnyFn
  focusedAgentId: AnyFn
  clearPendingPromptAttachments: AnyFn
  clearActiveToolLabels: AnyFn
  clearAgentPaneRuntime: AnyFn
  setDirectoryTreeState: AnyFn
  replaceTranscriptEntries: AnyFn
  applyResponseLayout: AnyFn
  setWorkspaceScreenMode: AnyFn
  resetPromptStop: AnyFn
  bumpHistoryLoadGeneration: AnyFn
  reconcileWaitingRoom: AnyFn
  refreshWaitingRoomData: AnyFn
  requestRootRender: AnyFn
  clearPromptInput: AnyFn
  blurPromptInput: AnyFn
  focusPromptInput: AnyFn
  setMultiAgentResponseLayout: AnyFn
  setAttachmentState: AnyFn
  setProviderRunState: AnyFn
  setCenterMode: AnyFn
  setCreatedSessionState: AnyFn
  setSessionState: AnyFn
  setProviderActivityLabel: AnyFn
  setActiveStatusLabel: AnyFn
  setAgentPaneEntries: AnyFn
  setAgentPanePreviews: AnyFn
  setAgentActivityLabels: AnyFn
  setStreamingAgentId: AnyFn
  setSubmitting: AnyFn
  setWorking: AnyFn
  setFatalError: AnyFn
  setDaemonDisconnected: AnyFn
  setNextHistoryCursor: AnyFn
  setSessionHydratingState: AnyFn
  setHistoryLoadingState: AnyFn
  setStatusLine: AnyFn
  setProviderCatalogState: AnyFn
  setTerminalCommandCatalogState: AnyFn
  availableSessions: AnyFn
  setAvailableSessions: AnyFn
  scheduleShortViewportHistoryCheck: AnyFn
  updateSessionChrome: AnyFn
  appendNotice: AnyFn
  flashFooter: AnyFn
  logProviderRunDebug: AnyFn
}

export function createCliSessionLifecycleComposition(deps: CliSessionLifecycleCompositionDeps) {
  const {
    hydrateCurrentAttachedSession,
    finalizeAttachedSessionBinding,
  } = createSessionAttachmentController({
    isAttached: deps.isAttached,
    attachmentState: deps.attachmentState,
    sessionState: deps.sessionState,
    getSessionState: (sessionId) => getSessionState(deps.client, sessionId),
    applySessionState: deps.applySessionState,
    refreshAgentPanes: deps.refreshAgentPanes,
    refreshSplitPaneFocusRepaint: deps.refreshSplitPaneFocusRepaint,
    maybeResize: (sessionId) => deps.maybeResize(sessionId),
    catchUpAttachedSession: (sessionId, attachmentId, session) =>
      deps.catchUpAttachedSession(sessionId, attachmentId, session),
    formatError: deps.formatError,
    logWarning: (message, fields) => {
      deps.appLogger?.warn(message, fields)
    },
  })

  const kernelEventSubscriptionController = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => deps.supportsKernelEventStream,
    getAttachment: deps.attachmentState,
    getSessionId: () => deps.sessionState().id,
    subscribeToWaitingRoomInventory: () => deps.client.subscribeToWaitingRoomInventory(),
    subscribeToKernelEvents: (sessionId, attachmentId) => deps.client.subscribeToKernelEvents(sessionId, attachmentId),
    onEvaluate: (state) => {
      deps.appLogger?.debug("evaluating kernel event subscription", {
        session_id: state.nextSessionId,
        attachment_id: state.nextAttachmentId,
        subscribed_session_id: state.sessionId,
        subscribed_attachment_id: state.attachmentId,
        subscribed_scope: state.scope,
        attached: state.attached,
      })
    },
    onWaitingRoomSubscribed: () => {
      deps.appLogger?.info("subscribed to waiting room inventory events")
    },
    onSessionSubscribed: (sessionId, attachmentId) => {
      deps.appLogger?.info("subscribed to kernel events", {
        session_id: sessionId,
        attachment_id: attachmentId,
      })
    },
    onWaitingRoomSubscriptionFailed: (error) => {
      deps.appLogger?.error("waiting room inventory subscription failed", {
        error: deps.formatError(error),
      })
      deps.setDaemonDisconnected(true)
      deps.setStatusLine("Waiting to reconnect to the Arroba kernel.")
      deps.appendNotice(`Waiting room inventory subscription failed: ${deps.formatError(error)}`, "warning")
      deps.updateSessionChrome()
    },
    onSessionSubscriptionFailed: (sessionId, attachmentId, error) => {
      deps.appLogger?.error("kernel event subscription failed", {
        session_id: sessionId,
        attachment_id: attachmentId,
        error: deps.formatError(error),
      })
      deps.setDaemonDisconnected(true)
      deps.setStatusLine("Waiting to reconnect to the Arroba kernel.")
      deps.appendNotice(`Kernel event subscription failed: ${deps.formatError(error)}`, "warning")
      deps.updateSessionChrome()
    },
  })
  const syncKernelEventSubscription = () => kernelEventSubscriptionController.sync()

  const kernelRestartRecoveryController = createKernelRestartRecoveryController({
    isClosing: deps.closingStateController.isClosing,
    isAttached: deps.isAttached,
    isDisconnected: deps.daemonDisconnected,
    getSessionId: () => deps.sessionState().id,
    getSessionState: (sessionId) => getSessionState(deps.client, sessionId),
    attachToSession: (sessionId) => attachToSession(deps.client, sessionId, deps.options.clientId),
    projectSession: (session) => applyProviderRunProfileToSession(session, deps.providerRunState()),
    applyAttachment: deps.setAttachmentState,
    applySession: deps.applySessionState,
    resetKernelEventSubscription: kernelEventSubscriptionController.reset,
    syncKernelEventSubscription,
    refreshAgentPanes: () => deps.refreshAgentPanes(deps.sessionState()),
    clearLocalBusyStateForAuthoritativeIdle: () => {
      deps.clearLocalBusyStateForAuthoritativeIdle(deps.sessionState())
    },
    onRecovered: () => {
      deps.recordDaemonActivity("kernel_restart_recovered")
      deps.setDaemonDisconnected(false)
      deps.setStatusLine(DEFAULT_CONNECTED_STATUS)
      deps.updateSessionChrome()
      deps.appendNotice("Reconnected to the Arroba kernel.")
    },
    onAttemptFailed: (sessionId, error) => {
      deps.appLogger?.debug("kernel restart recovery attempt failed", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
    sleep: deps.sleep,
  })
  const recoverAttachedSessionAfterKernelRestart = () => kernelRestartRecoveryController.recover()

  const {
    transitionToNoSession,
    detachCurrentAttachment,
    attachBinding,
  } = createSessionLifecycleController({
    cliOptions: deps.options,
    connectedStatus: DEFAULT_CONNECTED_STATUS,
    waitingRoomState: deps.waitingRoomState,
    attachmentState: deps.attachmentState,
    deriveDetachedCliTransitionState,
    deriveAttachedCliTransitionState,
    clearPendingPromptAttachments: deps.clearPendingPromptAttachments,
    clearActiveToolLabels: deps.clearActiveToolLabels,
    clearWorkflows: () => {},
    clearAgentPaneRuntime: deps.clearAgentPaneRuntime,
    clearDirectoryTree: () => deps.setDirectoryTreeState(null),
    clearTranscript: () => deps.replaceTranscriptEntries([]),
    refreshResponseLayout: deps.applyResponseLayout,
    resetWorkspaceScreen: () => deps.setWorkspaceScreenMode("agents"),
    resetStopRequestInFlight: deps.resetPromptStop,
    bumpHistoryLoadGeneration: deps.bumpHistoryLoadGeneration,
    reconcileWaitingRoom: deps.reconcileWaitingRoom,
    refreshWaitingRoomData: deps.refreshWaitingRoomData,
    requestRender: deps.requestRootRender,
    clearPromptInput: deps.clearPromptInput,
    syncPromptTextSnapshot: deps.syncPromptTextSnapshot,
    blurPromptInput: deps.blurPromptInput,
    focusPromptInput: deps.focusPromptInput,
    layoutPreference: () => deps.preferencesState().ui?.multiAgentResponseLayout ?? null,
    setMultiAgentResponseLayout: deps.setMultiAgentResponseLayout,
    setAttachmentState: deps.setAttachmentState,
    setProviderRunState: deps.setProviderRunState,
    setCenterMode: deps.setCenterMode,
    setCreatedSessionState: deps.setCreatedSessionState,
    setSessionState: deps.setSessionState,
    setProviderActivityLabel: deps.setProviderActivityLabel,
    setActiveStatusLabel: deps.setActiveStatusLabel,
    setAgentPaneEntries: deps.setAgentPaneEntries,
    setAgentPanePreviews: deps.setAgentPanePreviews,
    setAgentActivityLabels: deps.setAgentActivityLabels,
    setStreamingAgentId: deps.setStreamingAgentId,
    setSubmitting: deps.setSubmitting,
    setWorking: deps.setWorking,
    setFatalError: deps.setFatalError,
    setDaemonDisconnected: deps.setDaemonDisconnected,
    setNextHistoryCursor: deps.setNextHistoryCursor,
    setSessionHydratingState: deps.setSessionHydratingState,
    setHistoryLoadingState: deps.setHistoryLoadingState,
    setStatusLine: deps.setStatusLine,
    updateSessionChrome: deps.updateSessionChrome,
    refreshSplitPaneFocusRepaint: deps.refreshSplitPaneFocusRepaint,
    attachToSession: (sessionId, clientId) => attachToSession(deps.client, sessionId, clientId),
    getSessionState: (sessionId) => getSessionState(deps.client, sessionId),
    launchProviderRun: (sessionId, provider, accountProfile, model, effort, targetAgentId) =>
      launchProviderRun(deps.client, sessionId, provider, accountProfile, model, effort, targetAgentId),
    tryGetProviderRun: (providerRunId) => tryGetProviderRun(deps.client, providerRunId, deps.appLogger),
    setProviderCatalogState: deps.setProviderCatalogState,
    setTerminalCommandCatalogState: deps.setTerminalCommandCatalogState,
    getProviderCatalog: () => getProviderCatalog(deps.client, deps.appLogger),
    getTerminalCommandCatalog: () => getTerminalCommandCatalog(deps.client, deps.appLogger),
    syncCliProviderSelection: ({ provider, model, effort }) => {
      deps.options.provider = provider
      deps.options.model = model
      deps.options.effort = effort
      deps.reconcileWaitingRoom({
        ...deps.waitingRoomState(),
        providerId: normalizeBackendProviderId(provider),
        modelId: model,
        effort,
      })
    },
    primeAttachedSessionBinding: deps.primeAttachedSessionBinding,
    hydrateAttachedSessionBinding: (sessionId, attachmentId, session) =>
      finalizeAttachedSessionBinding({ sessionId, attachmentId, session }),
    getAvailableSessions: deps.availableSessions,
    setAvailableSessions: deps.setAvailableSessions,
    listSessions: () => listSessions(deps.client),
    scheduleShortViewportHistoryCheck: deps.scheduleShortViewportHistoryCheck,
    detachAttachment: (attachmentId) => detachSessionAttachment(deps.client, attachmentId),
    syncKernelEventSubscription,
    formatError: deps.formatError,
    logWarning: (message, fields) => {
      deps.appLogger?.warn(message, fields)
    },
    logAttachedProviderRun: (mode, run, fields) => {
      deps.logProviderRunDebug(
        mode === "launched"
          ? "attached session launched provider run"
          : "attached session loaded existing provider run",
        run,
        fields,
      )
    },
  })

  const providerRecoveryController = createProviderRecoveryController({
    isAttached: deps.isAttached,
    getSessionId: () => deps.sessionState().id,
    getSessionStateSnapshot: deps.sessionState,
    getFallbackLaunch: () => ({
      provider: deps.options.provider ?? "opencode",
      model: deps.currentModelId(),
      effort: deps.currentVariantId(),
    }),
    getAccountProfile: () => deps.options.accountProfile,
    getTargetAgentId: deps.focusedAgentId,
    launchProviderRun: ({ sessionId, provider, accountProfile, model, effort, targetAgentId }) =>
      launchProviderRun(deps.client, sessionId, provider, accountProfile, model, effort, targetAgentId),
    getSessionState: (sessionId) => getSessionState(deps.client, sessionId),
    projectSession: applyProviderRunProfileToSession,
    applyProviderRun: deps.setProviderRunState,
    applySession: deps.applySessionState,
    resizeSession: (sessionId) => deps.maybeResize(sessionId),
    onRecovered: (reason) => {
      deps.setStatusLine("Recovered provider connection.")
      deps.updateSessionChrome()
      deps.flashFooter(`recovered provider run after ${reason}`, "info")
    },
    onRecoverySkipped: (reason, skipReason) => {
      deps.appLogger?.warn("provider recovery skipped", {
        reason,
        skip_reason: skipReason,
      })
    },
    onRecoveryFailed: (reason, error) => {
      deps.appLogger?.warn("provider recovery failed", {
        reason,
        error: deps.formatError(error),
      })
    },
  })
  const recoverProviderRun = providerRecoveryController.recover

  const terminalExitController = createTerminalExitController({
    renderer: deps.renderer,
    sleep: deps.sleep,
    exitProcess: (exitCode) => process.exit(exitCode),
    onRendererDestroyFailed: (error) => {
      deps.appLogger?.warn("renderer destroy failed during exit", {
        error: deps.formatError(error),
      })
    },
  })
  const restoreTerminalAndExit = terminalExitController.restoreAndExit

  const exitController = createCliExitController({
    isClosing: deps.closingStateController.isClosing,
    setClosing: deps.closingStateController.setClosing,
    getCreatedSession: deps.createdSessionState,
    getConnectedClientCount: deps.connectedClientCount,
    getAttachment: deps.attachmentState,
    getSessionId: () => deps.sessionState().id,
    getPromptDraft: deps.persistablePromptDraft,
    syncPromptTextSnapshot: deps.syncPromptTextSnapshot,
    flushPromptDraftPersist: deps.flushPendingPromptDraftPersist,
    persistSessionPromptDraft: (sessionId, promptDraft) =>
      deps.persistSessionPromptState(sessionId, { promptDraft }),
    shouldEndSessionOnExit: shouldEndSessionOnCliExit,
    archiveSession: async (sessionId) => {
      await archiveSessionById(deps.client, sessionId)
    },
    detachAttachment: (attachmentId) => detachSessionAttachment(deps.client, attachmentId),
    getCleanupDecision: getExitCleanupDecision,
    restoreTerminalAndExit: (exitCode) => restoreTerminalAndExit(exitCode),
    onForceExitAfterCleanupFailure: () => {
      deps.appLogger?.warn("forcing cli exit after prior cleanup failure")
    },
    onExitRequested: (createdSession) => {
      deps.appLogger?.info("requested cli exit", {
        created_session: createdSession,
      })
    },
    onPromptDraftFlushFailed: (error) => {
      deps.appLogger?.warn("failed to flush prompt draft during exit", {
        error: deps.formatError(error),
      })
    },
    onPromptDraftPersistFailed: (sessionId, error) => {
      deps.appLogger?.warn("failed to persist prompt draft during exit", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
    onCleanupFailed: (decision, error) => {
      deps.appLogger?.warn("exit cleanup failed", {
        error: deps.formatError(error),
        will_exit: decision.exit,
      })
      deps.appendNotice(decision.message, "warning")
      deps.setStatusLine(decision.message)
    },
    onCleanupCompleted: () => {
      deps.appLogger?.info("cli exit cleanup completed")
    },
  })
  const requestExit = exitController.requestExit

  const waitingRoomTransitionController = createWaitingRoomTransitionController({
    isClosing: deps.closingStateController.isClosing,
    getCreatedSession: deps.createdSessionState,
    getConnectedClientCount: deps.connectedClientCount,
    getAttachment: deps.attachmentState,
    getSessionId: () => deps.sessionState().id,
    getPromptDraft: deps.persistablePromptDraft,
    syncPromptTextSnapshot: deps.syncPromptTextSnapshot,
    flushPromptDraftPersist: deps.flushPendingPromptDraftPersist,
    persistSessionPromptDraft: (sessionId, promptDraft) =>
      deps.persistSessionPromptState(sessionId, { promptDraft }),
    shouldEndSessionOnExit: shouldEndSessionOnCliExit,
    archiveSession: async (sessionId) => {
      await archiveSessionById(deps.client, sessionId)
    },
    detachAttachment: (attachmentId) => detachSessionAttachment(deps.client, attachmentId),
    transitionToWaitingRoom: (message) => {
      return transitionToNoSession(message)
    },
    onWaitingRoomRequested: (createdSession) => {
      deps.appLogger?.info("requested waiting room", {
        created_session: createdSession,
      })
    },
    onPromptDraftFlushFailed: (error) => {
      deps.appLogger?.warn("failed to flush prompt draft during waiting-room transition", {
        error: deps.formatError(error),
      })
    },
    onPromptDraftPersistFailed: (sessionId, error) => {
      deps.appLogger?.warn("failed to persist prompt draft during waiting-room transition", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
    onCleanupFailed: (error) => {
      deps.appLogger?.warn("waiting room cleanup failed", {
        error: deps.formatError(error),
      })
      deps.appendNotice(deps.formatError(error), "warning")
    },
    onTransitionCompleted: () => {
      deps.appLogger?.info("waiting room transition completed")
    },
  })
  const requestWaitingRoom = waitingRoomTransitionController.requestWaitingRoom

  return {
    hydrateCurrentAttachedSession,
    kernelEventSubscriptionController,
    syncKernelEventSubscription,
    recoverAttachedSessionAfterKernelRestart,
    transitionToNoSession,
    detachCurrentAttachment,
    attachBinding,
    recoverProviderRun,
    requestExit,
    requestWaitingRoom,
    restoreTerminalAndExit,
  }
}
