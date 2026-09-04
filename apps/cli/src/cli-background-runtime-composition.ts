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
import {
  DEFAULT_CONNECTED_STATUS,
  SILENT_POLL_THRESHOLD,
  getPollRecoveryDecision,
} from "./runtime.js"
import { getProviderActivityLabel } from "@chariox/kernel-client/provider-status"
import {
  applyProviderRunProfileToSession,
} from "@chariox/kernel-client/prompt-provider-selection"
import { isSessionUnavailableError } from "./session-errors.js"
import { createTerminalResizeController } from "./terminal-resize-controller.js"
import {
  previewLineForTerminalRecord,
} from "@chariox/kernel-client/session-history-preview"
import { sameProviderRun } from "@chariox/kernel-client/session-runtime-lookup"
import { createTranscriptScrollMonitorController } from "./transcript-scroll-monitor-controller.js"
import {
  computeNextTranscriptTurnId as computeNextTurnId,
  trimSingleTrailingNewline,
} from "@chariox/kernel-client/transcript-entry-state"
import {
  shouldRenderProviderStatus,
} from "./transcript.js"
import type { WorkflowDesignOpForwarded } from "@chariox/kernel-client/kernel-types"
import type {
  AgentRuntimeActivity,
  RuntimeInteraction,
  RuntimeProviderRun,
  RuntimeSession,
  WorkflowRun,
} from "./cli-types.js"
import { normalizeRuntimeSessionWithAgentActivity } from "./cli-types.js"
import { createWaitingRoomIntroAnimationController } from "./waiting-room-intro-animation-controller.js"
import { createWaitingRoomRefreshIntervalController } from "./waiting-room-refresh-interval-controller.js"
import { createWorkingAnimationController } from "./working-animation-controller.js"
import { workflowsWithDesignOp } from "./workflow-design-op-state.js"
import {
  createRoomEnvironmentActivityController,
} from "./room-environment-activity-controller.js"
import { workflowRuntimeSignature } from "./workflow-runtime-signature.js"

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
  workflowScreenActive: AnyFn
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
  refreshAgentHistories: AnyFn
  attachmentState: AnyFn
  catchUpAttachedSession: AnyFn
  getSessionState: AnyFn
  getWorkspaceLiveSyncStatus?: AnyFn
  setWorkspaceLiveSyncStatus?: AnyFn
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
  drainTerminalOutputRecords: AnyFn
  scheduleSharedPromptInputHistoryRefresh: AnyFn
  handleWaitingRoomRefresh: AnyFn
  applyWaitingRoomRowsChanged: AnyFn
  applyRelayStatusChanged: AnyFn
  applyRemoteMachinesChanged: AnyFn
  applyProviderCatalogChanged: AnyFn
  applySlicesChanged: AnyFn
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
  const roomEnvironmentActivityController = createRoomEnvironmentActivityController({
    isAttached: deps.isAttached,
    sessionId: () => deps.sessionState().id,
    nowMs: Date.now,
    send: (request) => deps.client.send(request),
    appendNotice: (message, key) => deps.appendNotice(message, "muted", key),
    recordDaemonActivity,
  })

  const refreshAssistantMessageHistory = (agentId: string) => {
    if (!deps.isAttached()) {
      return
    }
    const session = deps.sessionState()
    void deps.refreshAgentHistories(session, [agentId]).then(() => {
      if (!deps.isAttached() || deps.sessionState().id !== session.id) {
        return
      }
      deps.syncVisibleTranscriptPreview()
      deps.appLogger?.debug?.("refreshed completed assistant history", {
        session_id: session.id,
        agent_id: agentId,
      })
    }).catch((error: unknown) => {
      deps.appLogger?.warn?.("failed to refresh completed assistant history", {
        session_id: session.id,
        agent_id: agentId,
        error: deps.formatError(error),
      })
    })
  }

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
    handleExternalProviderHistoryUpdated: (agentId) => {
      void (async () => {
        if (!deps.isAttached()) {
          return
        }
        const currentSession = deps.sessionState()
        const latestSession = await deps.getSessionState(currentSession.id)
        deps.applySessionState(latestSession)
        await deps.refreshAgentPanes(latestSession)
        deps.syncVisibleTranscriptPreview()
        deps.appLogger?.debug?.("refreshed external provider history", {
          session_id: latestSession.id,
          agent_id: agentId,
        })
      })().catch((error) => {
        deps.appLogger?.warn?.("failed to refresh external provider history", {
          agent_id: agentId,
          error: deps.formatError(error),
        })
      })
    },
  })

  const kernelSessionSnapshotController = createKernelSessionSnapshotController({
    getSession: deps.sessionState,
    getProviderRun: deps.providerRunState,
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange: deps.shouldRefreshAgentPanesForSessionChange,
    applySessionState: deps.applySessionState,
    sameProviderRun,
    logProviderRunDebug: deps.logProviderRunDebug,
    setProviderRun: deps.setProviderRunState,
    updateSessionChrome: deps.updateSessionChrome,
    supportsKernelEventStream: () => deps.supportsKernelEventStream,
    recoverProviderRun: deps.recoverProviderRun,
    refreshAgentPanes: deps.refreshAgentPanes,
  })
  const applyKernelSessionSnapshotWithWorkflowRefresh = async (
    nextSession: RuntimeSession,
    nextProviderRun: RuntimeProviderRun | null,
  ) => {
    const shouldTrackWorkflowOutline = deps.workflowScreenActive()
    const previousSignature = shouldTrackWorkflowOutline
      ? workflowRuntimeSignature(deps.sessionState())
      : null
    await kernelSessionSnapshotController.apply(nextSession, nextProviderRun)
    if (
      shouldTrackWorkflowOutline
      && workflowRuntimeSignature(deps.sessionState()) !== previousSignature
    ) {
      deps.rebuildTranscript()
    }
  }

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
      deps.setStatusLine("Waiting to reconnect to the Chariox kernel.")
      deps.updateSessionChrome()
    },
  })

  const resyncAttachedKernelState = (reason: string) => kernelResyncController.resync(reason)
  const applyDeltaSessionState = (sessionId: string, apply: (session: RuntimeSession) => RuntimeSession) => {
    if (!deps.isAttached() || deps.sessionState().id !== sessionId) {
      return false
    }
    const nextSession = apply(deps.sessionState())
    deps.applySessionState(nextSession)
    deps.clearLocalBusyStateForAuthoritativeIdle(nextSession)
    deps.updateSessionChrome()
    return true
  }
  const applyAgentActivityChanged = (
    sessionId: string,
    agentActivity: Record<string, unknown>,
    agentActivityRevision: number | null,
  ) => {
    applyDeltaSessionState(sessionId, (session) => normalizeRuntimeSessionWithAgentActivity({
      session,
      agent_activity: agentActivity as Record<string, AgentRuntimeActivity>,
      agent_activity_revision: agentActivityRevision,
    }))
  }
  const applyProviderRunChanged = (
    sessionId: string,
    providerRun: RuntimeProviderRun | null,
  ) => {
    if (!deps.isAttached() || deps.sessionState().id !== sessionId) {
      return
    }
    if (providerRun && providerRun.session_id !== sessionId) {
      deps.appendNotice(`Kernel sent provider run ${providerRun.id} for session ${providerRun.session_id}, expected ${sessionId}.`, "warning")
      return
    }
    deps.setProviderRunState(providerRun)
    applyDeltaSessionState(sessionId, (session) => applyProviderRunProfileToSession(session, providerRun))
  }
  const applySessionMetadataChanged = (
    sessionId: string,
    metadata: Record<string, unknown>,
  ) => {
    applyDeltaSessionState(sessionId, (session) => {
      const patch: Partial<RuntimeSession> = {}
      if (Object.prototype.hasOwnProperty.call(metadata, "alias")) {
        patch.alias = nullableString(metadata.alias)
      }
      if (Object.prototype.hasOwnProperty.call(metadata, "last_used_at_ms")) {
        patch.last_used_at_ms = nullableNumber(metadata.last_used_at_ms)
      }
      if (Object.prototype.hasOwnProperty.call(metadata, "last_prompt_sent_at_ms")) {
        patch.last_prompt_sent_at_ms = nullableNumber(metadata.last_prompt_sent_at_ms)
      }
      if (Object.prototype.hasOwnProperty.call(metadata, "hidden")) {
        patch.hidden = metadata.hidden === true
      }
      if (Object.prototype.hasOwnProperty.call(metadata, "focused_agent_id")) {
        patch.focused_agent_id = nullableString(metadata.focused_agent_id)
      }
      if (Object.prototype.hasOwnProperty.call(metadata, "workspace_live_sync_mode")) {
        patch.workspace_live_sync_mode = workspaceLiveSyncMode(metadata.workspace_live_sync_mode)
      }
      return { ...session, ...patch }
    })
  }
  const applyRuntimeInteractionsChanged = (
    sessionId: string,
    activeInteractions: Record<string, unknown>[],
  ) => {
    applyDeltaSessionState(sessionId, (session) => ({
      ...session,
      active_interactions: activeInteractions as RuntimeInteraction[],
    }))
  }
  const applyWorkflowRunUpdated = (
    sessionId: string,
    workflowRun: WorkflowRun,
  ) => {
    const applied = applyDeltaSessionState(sessionId, (session) => {
      const existingRuns = session.workflow_runs ?? []
      const index = existingRuns.findIndex((run) => run.id === workflowRun.id)
      const workflowRuns = index === -1
        ? [...existingRuns, workflowRun]
        : existingRuns.map((run, runIndex) => runIndex === index ? workflowRun : run)
      return {
        ...session,
        workflow_runs: workflowRuns,
      }
    })
    if (applied && deps.workflowScreenActive()) {
      deps.rebuildTranscript()
    }
  }
  const applyWorkflowDesignOp = (event: WorkflowDesignOpForwarded) => {
    const applied = applyDeltaSessionState(event.session_id, (session) => ({
      ...session,
      workflows: workflowsWithDesignOp(session.workflows ?? [], event.op),
    }))
    if (applied && deps.workflowScreenActive()) {
      deps.rebuildTranscript()
    }
  }

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
    drainTerminalOutputRecords: deps.drainTerminalOutputRecords,
    applyRuntimeNotices: kernelEventController.applyRuntimeNotices,
    applyAssistantMessageCompleted: kernelEventController.applyAssistantMessageCompleted,
    refreshAssistantMessageHistory,
    applyKernelSessionSnapshot: applyKernelSessionSnapshotWithWorkflowRefresh,
    applyAgentActivityChanged,
    applyProviderRunChanged,
    applySessionMetadataChanged,
    applyRuntimeInteractionsChanged,
    applyWorkflowRunUpdated,
    applyWorkflowDesignOp,
    scheduleSharedPromptInputHistoryRefresh: deps.scheduleSharedPromptInputHistoryRefresh,
    handleKernelSessionUnavailable,
    refreshWaitingRoomData: deps.handleWaitingRoomRefresh,
    applyWaitingRoomRowsChanged: deps.applyWaitingRoomRowsChanged,
    applyRelayStatusChanged: deps.applyRelayStatusChanged,
    applyRemoteMachinesChanged: deps.applyRemoteMachinesChanged,
    applyProviderCatalogChanged: deps.applyProviderCatalogChanged,
    applySlicesChanged: deps.applySlicesChanged,
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
    workflowScreenActive: deps.workflowScreenActive,
    rebuildTranscript: deps.rebuildTranscript,
    getProviderRun: deps.providerRunState,
    setProviderRun: deps.setProviderRunState,
    updateSessionChrome: deps.updateSessionChrome,
    recordDaemonActivity,
    queueTerminalOutputRecords: deps.queueTerminalOutputRecords,
    pumpTerminalOutput: deps.pumpTerminalOutput,
    pollRuntimeNotices: deps.pollRuntimeNotices,
    synchronizeRoomEnvironmentActivity: roomEnvironmentActivityController.synchronize,
    appendNotice: (message) => deps.appendNotice(message),
    getSessionState: deps.getSessionState,
    ...(deps.getWorkspaceLiveSyncStatus && deps.setWorkspaceLiveSyncStatus
      ? {
        getWorkspaceLiveSyncStatus: deps.getWorkspaceLiveSyncStatus,
        setWorkspaceLiveSyncStatus: deps.setWorkspaceLiveSyncStatus,
      }
      : {}),
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
  const pollRoomEnvironmentActivity = pollingController.pollRoomEnvironmentActivity

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
    pollRoomEnvironmentActivity,
    startConnectionWatchdog,
    stopConnectionWatchdog: () => {
      connectionHealthWatchdogController.stop()
    },
    logViewDebug: deps.logViewDebug,
  })
  const ensureBackgroundPollersStarted = backgroundPollerStartupController.ensureStarted

  onCleanup(() => {
    deps.closingStateController.markClosing()
    roomEnvironmentActivityController.reset()
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

function nullableString(value: unknown): string | null {
  return typeof value === "string" ? value : null
}

function nullableNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function workspaceLiveSyncMode(value: unknown): Exclude<RuntimeSession["workspace_live_sync_mode"], undefined> {
  return value === "managed" || value === "tracked" || value === "unrestricted"
    ? value
    : null
}
