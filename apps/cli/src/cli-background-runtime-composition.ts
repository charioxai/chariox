import { createEffect, onCleanup, onMount } from "solid-js"

import { createBackgroundPollerStartupController } from "./background-poller-startup-controller.js"
import { createCliPollingController } from "./cli-polling-controller.js"
import {
  createConnectionHealthWatchdogController,
} from "./connection-health-watchdog-controller.js"
import { createDaemonActivityController } from "./daemon-activity-controller.js"
import { createKernelEventDispatchController } from "./kernel-event-dispatch-controller.js"
import { createKernelEventController } from "./kernel-event-controller.js"
import { createKernelResyncController } from "./kernel-resync-controller.js"
import { createKernelSessionSnapshotController } from "./kernel-session-snapshot-controller.js"
import { createKernelSessionUnavailableController } from "./kernel-session-unavailable-controller.js"
import { createPollerDegradationController } from "./poller-degradation-controller.js"
import { sameProviderRun } from "./provider-api.js"
import {
  DEFAULT_CONNECTED_STATUS,
  SILENT_POLL_THRESHOLD,
  getPollRecoveryDecision,
  getProviderActivityLabel,
} from "./runtime.js"
import {
  applyProviderRunProfileToSession,
} from "./session-chrome-state.js"
import { sessionHasPromptWork } from "./session-state.js"
import { isSessionUnavailableError } from "./session-errors.js"
import { createTerminalResizeController } from "./terminal-resize-controller.js"
import {
  computeNextTurnId,
  previewLineForTerminalRecord,
} from "./transcript-preview.js"
import { createTranscriptScrollMonitorController } from "./transcript-scroll-monitor-controller.js"
import {
  shouldRenderProviderStatus,
} from "./transcript.js"
import {
  trimSingleTrailingNewline,
} from "./transcript-text.js"
import { createWaitingRoomIntroAnimationController } from "./waiting-room-intro-animation-controller.js"
import { createWaitingRoomRefreshIntervalController } from "./waiting-room-refresh-interval-controller.js"
import { createWorkingAnimationController } from "./working-animation-controller.js"

type AnyFn = (...args: any[]) => any

export type CliBackgroundRuntimeCompositionDeps = {
  client: any
  appLogger: any
  formatError: AnyFn
  sleep: AnyFn
  scheduleInterval: AnyFn
  clearInterval: AnyFn
  closingStateController: {
    isClosing: AnyFn
    markClosing: AnyFn
  }
  isAttached: AnyFn
  sessionState: AnyFn
  resizeSession: AnyFn
  setDaemonDisconnected: AnyFn
  setStatusLine: AnyFn
  updateSessionChrome: AnyFn
  appendNotice: AnyFn
  working: AnyFn
  supportsKernelEventStream: boolean
  recoverProviderRun: AnyFn
  daemonDisconnected: AnyFn
  recordTurnActivity: AnyFn
  resolveTerminalRecordAgentId: AnyFn
  setStreamingAgentId: AnyFn
  markAgentBusy: AnyFn
  splitAgentResponseMode: AnyFn
  visibleTranscriptAgentId: AnyFn
  focusedAgentId: AnyFn
  hasTrailingUserPrompt: AnyFn
  currentAgentPaneEntries: AnyFn
  appendTranscriptEntryToAgentPane: AnyFn
  appendProviderChunkToAgentPane: AnyFn
  appendToolUpdateToAgentPane: AnyFn
  setAgentActivityLabel: AnyFn
  agentActivityLabel: AnyFn
  setProviderActivityLabel: AnyFn
  applyProviderActivity: AnyFn
  syncVisibleActivityLabel: AnyFn
  appendEntry: AnyFn
  appendProviderChunk: AnyFn
  appendToolUpdate: AnyFn
  appendProviderError: AnyFn
  syncVisibleTranscriptPreview: AnyFn
  appendAgentPanePreview: AnyFn
  markAssistantMessageCompleted: AnyFn
  providerRunState: AnyFn
  shouldRefreshAgentPanesForSessionChange: AnyFn
  applySessionState: AnyFn
  logProviderRunDebug: AnyFn
  setProviderRunState: AnyFn
  refreshAgentPanes: AnyFn
  attachmentState: AnyFn
  catchUpAttachedSession: AnyFn
  getSessionState: AnyFn
  tryGetProviderRun: AnyFn
  clearLocalBusyStateForAuthoritativeIdle: AnyFn
  attachToSession: AnyFn
  setAttachmentState: AnyFn
  kernelEventSubscriptionController: {
    reset: AnyFn
  }
  syncKernelEventSubscription: AnyFn
  transitionToNoSession: AnyFn
  queueTerminalOutputRecords: AnyFn
  scheduleSharedPromptInputHistoryRefresh: AnyFn
  handleWaitingRoomRefresh: AnyFn
  flashFooter: AnyFn
  recoverAttachedSessionAfterKernelRestart: AnyFn
  setFatalError: AnyFn
  pumpTerminalOutput: AnyFn
  pollRuntimeNotices: AnyFn
  promptInputRefController: {
    hasInput: AnyFn
    focus: AnyFn
    blur: AnyFn
  }
  transcriptScrollboxRefController: {
    hasScrollbox: AnyFn
    scrollTop: AnyFn
  }
  primaryTranscriptRuntimeStore: {
    setLastScrollTop: AnyFn
  }
  rebuildTranscript: AnyFn
  syncPromptPlaceholder: AnyFn
  addResizeListener: AnyFn
  removeResizeListener: AnyFn
  logViewDebug: AnyFn
  footerFlashController: {
    clearTimer: AnyFn
  }
  clearPendingPromptDraftPersist: AnyFn
  cancelPendingTurnCompletion: AnyFn
  sessionChromeUpdateController: {
    clearTimer: AnyFn
  }
  promptInputHistoryRefreshController: {
    clearTimer: AnyFn
  }
  transcriptHistoryAutoloadController: {
    monitorScroll: AnyFn
  }
  setWorkingAnimationFrame: AnyFn
  sessionStatusMode: AnyFn
  renderSplitPaneFooters: AnyFn
  waitingRoomState: AnyFn
  setWaitingRoomState: AnyFn
  kernelConnected: AnyFn
  hydrateCurrentAttachedSession: AnyFn
}

export function createCliBackgroundRuntimeComposition(deps: CliBackgroundRuntimeCompositionDeps) {
  const terminalResizeController = createTerminalResizeController({
    isAttached: deps.isAttached,
    sessionId: () => deps.sessionState().id,
    resizeSession: deps.resizeSession,
  })
  const onResize = terminalResizeController.handleResize

  const pollerDegradationController = createPollerDegradationController({
    connectedStatusLine: DEFAULT_CONNECTED_STATUS,
    logger: deps.appLogger,
    setDaemonDisconnected: deps.setDaemonDisconnected,
    setStatusLine: deps.setStatusLine,
    updateSessionChrome: deps.updateSessionChrome,
    appendNotice: deps.appendNotice,
  })
  const markPollerDegraded = pollerDegradationController.markDegraded
  const markPollerRecovered = pollerDegradationController.markRecovered

  const connectionHealthWatchdogController = createConnectionHealthWatchdogController({
    now: Date.now,
    intervalMs: 250,
    silenceWindowMs: 2000,
    silentThreshold: SILENT_POLL_THRESHOLD,
    scheduleInterval: deps.scheduleInterval,
    clearInterval: deps.clearInterval,
    isClosing: deps.closingStateController.isClosing,
    isAttached: deps.isAttached,
    isWorking: deps.working,
    onRecover: (decision) => {
      deps.appLogger?.warn?.("connection appears stale - no activity while working", {
        consecutive_silent_polls: decision.nextConsecutiveSilentPolls,
        time_since_last_activity_ms: decision.timeSinceLastActivityMs,
      })
      if (deps.supportsKernelEventStream) {
        void deps.client.restartKernelEventStream().catch((error: unknown) => {
          deps.appLogger?.warn?.("kernel event stream restart failed", {
            error: deps.formatError(error),
          })
        })
      } else {
        void deps.recoverProviderRun("stale connection - no activity received")
      }
    },
  })

  const daemonActivityController = createDaemonActivityController({
    recordConnectionActivity: () => connectionHealthWatchdogController.recordActivity(),
    daemonDisconnected: deps.daemonDisconnected,
    setDaemonDisconnected: deps.setDaemonDisconnected,
    updateSessionChrome: deps.updateSessionChrome,
  })
  const recordDaemonActivity = daemonActivityController.record

  const kernelEventController = createKernelEventController({
    recordDaemonActivity,
    recordTurnActivity: deps.recordTurnActivity,
    resolveTerminalRecordAgentId: deps.resolveTerminalRecordAgentId,
    setStreamingAgentId: deps.setStreamingAgentId,
    markAgentBusy: deps.markAgentBusy,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    visibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    focusedAgentId: deps.focusedAgentId,
    hasTrailingUserPrompt: deps.hasTrailingUserPrompt,
    currentAgentPaneEntries: deps.currentAgentPaneEntries,
    computeNextTurnId,
    appendTranscriptEntryToAgentPane: deps.appendTranscriptEntryToAgentPane,
    appendProviderChunkToAgentPane: deps.appendProviderChunkToAgentPane,
    appendToolUpdateToAgentPane: deps.appendToolUpdateToAgentPane,
    setAgentActivityLabel: deps.setAgentActivityLabel,
    agentActivityLabel: deps.agentActivityLabel,
    setProviderActivityLabel: deps.setProviderActivityLabel,
    applyProviderActivity: deps.applyProviderActivity,
    syncVisibleActivityLabel: deps.syncVisibleActivityLabel,
    getProviderActivityLabel,
    shouldRenderProviderStatus,
    appendEntry: deps.appendEntry,
    appendProviderChunk: deps.appendProviderChunk,
    appendToolUpdate: deps.appendToolUpdate,
    appendProviderError: deps.appendProviderError,
    syncVisibleTranscriptPreview: deps.syncVisibleTranscriptPreview,
    appendAgentPanePreview: deps.appendAgentPanePreview,
    previewLineForTerminalRecord,
    trimSingleTrailingNewline,
    setDaemonDisconnected: deps.setDaemonDisconnected,
    setStatusLine: deps.setStatusLine,
    updateSessionChrome: deps.updateSessionChrome,
    appendNotice: (message, tone) => deps.appendNotice(message, tone === "warning" ? "warning" : "muted"),
    connectedStatusLine: DEFAULT_CONNECTED_STATUS,
    markAssistantMessageCompleted: deps.markAssistantMessageCompleted,
  })

  const kernelSessionSnapshotController = createKernelSessionSnapshotController({
    getSession: deps.sessionState,
    getProviderRun: deps.providerRunState,
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange: deps.shouldRefreshAgentPanesForSessionChange,
    sessionHasPromptWork,
    applySessionState: deps.applySessionState,
    sameProviderRun,
    logProviderRunDebug: deps.logProviderRunDebug,
    setProviderRun: deps.setProviderRunState,
    updateSessionChrome: deps.updateSessionChrome,
    supportsKernelEventStream: () => deps.supportsKernelEventStream,
    recoverProviderRun: deps.recoverProviderRun,
    refreshAgentPanes: deps.refreshAgentPanes,
  })
  const applyKernelSessionSnapshot = kernelSessionSnapshotController.apply

  const kernelResyncController = createKernelResyncController({
    getAttachment: deps.attachmentState,
    isAttached: deps.isAttached,
    getSessionId: () => deps.sessionState().id,
    getSessionStateSnapshot: deps.sessionState,
    catchUpAttachedSession: deps.catchUpAttachedSession,
    getSessionState: deps.getSessionState,
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: deps.providerRunState,
    tryGetProviderRun: deps.tryGetProviderRun,
    sameProviderRun,
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange: deps.shouldRefreshAgentPanesForSessionChange,
    sessionHasPromptWork,
    applySession: deps.applySessionState,
    applyProviderRun: deps.setProviderRunState,
    refreshAgentPanes: deps.refreshAgentPanes,
    clearLocalBusyStateForAuthoritativeIdle: deps.clearLocalBusyStateForAuthoritativeIdle,
    onProviderRunCleared: (run, sessionId, reason) => {
      deps.logProviderRunDebug("kernel resync cleared provider run", run, {
        session_id: sessionId,
        reason,
      })
    },
    onProviderRunRefreshed: (run, sessionId, previousProviderRunId, reason) => {
      deps.logProviderRunDebug("kernel resync refreshed provider run", run, {
        session_id: sessionId,
        previous_provider_run_id: previousProviderRunId,
        reason,
      })
    },
    onResyncStart: (sessionId, attachmentId, reason) => {
      deps.appLogger?.info?.("resyncing attached kernel state", {
        reason,
        session_id: sessionId,
        attachment_id: attachmentId,
      })
    },
    onResyncComplete: (reason) => {
      recordDaemonActivity(`kernel_resync_${reason}`)
      deps.setDaemonDisconnected(false)
      deps.setStatusLine(DEFAULT_CONNECTED_STATUS)
      deps.updateSessionChrome()
    },
    onResyncFailed: (reason, error) => {
      deps.appLogger?.warn?.("attached kernel resync failed", {
        reason,
        error: deps.formatError(error),
      })
      deps.setDaemonDisconnected(true)
      deps.setStatusLine("Waiting to reconnect to the Arroba kernel.")
      deps.updateSessionChrome()
    },
  })

  const resyncAttachedKernelState = (reason: string) => kernelResyncController.resync(reason)

  const kernelSessionUnavailableController = createKernelSessionUnavailableController({
    isAttached: deps.isAttached,
    getSession: deps.sessionState,
    getProviderRun: deps.providerRunState,
    getSessionState: deps.getSessionState,
    attachToSession: deps.attachToSession,
    applyAttachment: deps.setAttachmentState,
    projectSession: applyProviderRunProfileToSession,
    applySession: deps.applySessionState,
    resetKernelEventSubscription: deps.kernelEventSubscriptionController.reset,
    syncKernelEventSubscription: deps.syncKernelEventSubscription,
    refreshAgentPanes: deps.refreshAgentPanes,
    clearLocalBusyStateForAuthoritativeIdle: deps.clearLocalBusyStateForAuthoritativeIdle,
    recordDaemonActivity,
    onRecovered: () => {
      deps.setDaemonDisconnected(false)
      deps.setStatusLine(DEFAULT_CONNECTED_STATUS)
      deps.updateSessionChrome()
    },
    onStateLookupFailed: (sessionId, message, error) => {
      deps.appLogger?.debug?.("session unavailable confirmed by state lookup failure", {
        session_id: sessionId,
        message,
        error: deps.formatError(error),
      })
    },
    transitionToNoSession: deps.transitionToNoSession,
  })
  const handleKernelSessionUnavailable = kernelSessionUnavailableController.handle

  const kernelEventDispatchController = createKernelEventDispatchController({
    recordDaemonActivity,
    queueTerminalOutputRecords: deps.queueTerminalOutputRecords,
    applyRuntimeNotices: kernelEventController.applyRuntimeNotices,
    applyAssistantMessageCompleted: kernelEventController.applyAssistantMessageCompleted,
    applyKernelSessionSnapshot,
    scheduleSharedPromptInputHistoryRefresh: deps.scheduleSharedPromptInputHistoryRefresh,
    handleKernelSessionUnavailable,
    refreshWaitingRoomData: deps.handleWaitingRoomRefresh,
    applyTransportResumed: kernelEventController.applyTransportResumed,
    resyncAttachedKernelState,
    appendNotice: deps.appendNotice,
    flashFooter: deps.flashFooter,
    applyTransportClosed: kernelEventController.applyTransportClosed,
    recoverAttachedSessionAfterKernelRestart: deps.recoverAttachedSessionAfterKernelRestart,
  })
  const handleKernelEvent = kernelEventDispatchController.handleKernelEvent

  const startConnectionWatchdog = connectionHealthWatchdogController.start

  const pollingController = createCliPollingController({
    isClosing: deps.closingStateController.isClosing,
    logger: deps.appLogger,
    formatError: deps.formatError,
    isSessionUnavailableError,
    getPollRecoveryDecision,
    onSessionUnavailable: () => {
      deps.transitionToNoSession("Current session is no longer available.")
    },
    onMarkRecovered: markPollerRecovered,
    onMarkDegraded: markPollerDegraded,
    onFatalError: (error) => {
      if (error instanceof Error && /local transport/i.test(error.message)) {
        deps.setDaemonDisconnected(true)
      }
      deps.setFatalError(deps.formatError(error))
      deps.updateSessionChrome()
    },
    sleep: deps.sleep,
    isAttached: deps.isAttached,
    getAttachment: deps.attachmentState,
    getSession: deps.sessionState,
    getProviderRun: deps.providerRunState,
    setProviderRun: deps.setProviderRunState,
    updateSessionChrome: deps.updateSessionChrome,
    recordDaemonActivity,
    queueTerminalOutputRecords: deps.queueTerminalOutputRecords,
    pumpTerminalOutput: deps.pumpTerminalOutput,
    pollRuntimeNotices: deps.pollRuntimeNotices,
    appendNotice: (message) => deps.appendNotice(message),
    sessionHasPromptWork,
    getSessionState: deps.getSessionState,
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange: deps.shouldRefreshAgentPanesForSessionChange,
    applySessionState: deps.applySessionState,
    refreshAgentPanes: deps.refreshAgentPanes,
    tryGetProviderRun: deps.tryGetProviderRun,
    sameProviderRun,
    logProviderRunDebug: deps.logProviderRunDebug,
    recoverProviderRun: deps.recoverProviderRun,
  })
  const pollOutput = pollingController.pollOutput
  const pollNotices = pollingController.pollNotices
  const pollSessionState = pollingController.pollSessionState

  const backgroundPollerStartupController = createBackgroundPollerStartupController({
    logger: deps.appLogger,
    ready: () => deps.promptInputRefController.hasInput() && deps.transcriptScrollboxRefController.hasScrollbox(),
    promptMounted: deps.promptInputRefController.hasInput,
    transcriptScrollTop: () => deps.transcriptScrollboxRefController.scrollTop(0),
    setLastTranscriptScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
    isAttached: deps.isAttached,
    rebuildTranscript: deps.rebuildTranscript,
    syncPromptPlaceholder: deps.syncPromptPlaceholder,
    focusPrompt: () => {
      deps.promptInputRefController.focus()
    },
    blurPrompt: () => {
      deps.promptInputRefController.blur()
    },
    addResizeListener: () => {
      deps.addResizeListener(onResize)
    },
    removeResizeListener: () => {
      deps.removeResizeListener(onResize)
    },
    supportsKernelEventStream: () => deps.supportsKernelEventStream,
    syncKernelEventSubscription: deps.syncKernelEventSubscription,
    pollOutput,
    pollNotices,
    pollSessionState,
    startConnectionWatchdog,
    stopConnectionWatchdog: () => {
      connectionHealthWatchdogController.stop()
    },
    logViewDebug: deps.logViewDebug,
  })
  const ensureBackgroundPollersStarted = backgroundPollerStartupController.ensureStarted

  onCleanup(() => {
    deps.closingStateController.markClosing()
    backgroundPollerStartupController.stop()
  })

  onCleanup(() => {
    deps.footerFlashController.clearTimer()
    deps.clearPendingPromptDraftPersist()
    deps.cancelPendingTurnCompletion()
    deps.sessionChromeUpdateController.clearTimer()
    deps.promptInputHistoryRefreshController.clearTimer()
  })

  const disposeKernelEventHandler = deps.supportsKernelEventStream
    ? deps.client.onKernelEvent((event: any) => {
      void handleKernelEvent(event)
    })
    : () => {}

  createEffect(() => {
    void deps.attachmentState()
    void deps.sessionState().id
    void deps.syncKernelEventSubscription()
  })

  onCleanup(() => {
    disposeKernelEventHandler()
    void deps.client.unsubscribeFromKernelEvents().catch(() => {
      // Ignore teardown errors while closing the TUI.
    })
  })

  const transcriptScrollMonitorController = createTranscriptScrollMonitorController({
    intervalMs: 75,
    scheduleInterval: deps.scheduleInterval,
    clearInterval: deps.clearInterval,
    monitorScroll: () => {
      deps.transcriptHistoryAutoloadController.monitorScroll()
    },
  })
  transcriptScrollMonitorController.start()

  onCleanup(() => {
    transcriptScrollMonitorController.stop()
  })

  const workingAnimationController = createWorkingAnimationController({
    intervalMs: 120,
    scheduleInterval: deps.scheduleInterval,
    clearInterval: deps.clearInterval,
    incrementFrame: () => {
      deps.setWorkingAnimationFrame((value: number) => value + 1)
    },
    sessionStatusMode: deps.sessionStatusMode,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    updateSessionChrome: deps.updateSessionChrome,
    renderSplitPaneFooters: deps.renderSplitPaneFooters,
  })
  workingAnimationController.start()

  onCleanup(() => {
    workingAnimationController.stop()
  })

  const waitingRoomIntroAnimationController = createWaitingRoomIntroAnimationController({
    intervalMs: 90,
    scheduleInterval: deps.scheduleInterval,
    clearInterval: deps.clearInterval,
    isAttached: deps.isAttached,
    getWaitingRoomState: deps.waitingRoomState,
    setWaitingRoomState: deps.setWaitingRoomState,
    rebuildTranscript: deps.rebuildTranscript,
  })
  waitingRoomIntroAnimationController.start()

  onCleanup(() => {
    waitingRoomIntroAnimationController.stop()
  })

  const waitingRoomRefreshIntervalController = createWaitingRoomRefreshIntervalController({
    intervalMs: 2_500,
    scheduleInterval: deps.scheduleInterval,
    clearInterval: deps.clearInterval,
    refreshWaitingRoomData: deps.handleWaitingRoomRefresh,
  })
  waitingRoomRefreshIntervalController.start()

  onCleanup(() => {
    waitingRoomRefreshIntervalController.stop()
  })

  onMount(() => {
    if (deps.kernelConnected()) {
      void deps.handleWaitingRoomRefresh()
      void deps.hydrateCurrentAttachedSession("mount")
    }
  })

  return {
    ensureBackgroundPollersStarted,
    processKernelTerminalOutputRecord: kernelEventController.processTerminalOutputRecord,
    recordDaemonActivity,
  }
}
