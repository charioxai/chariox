import process from "node:process"
import { randomBytes } from "node:crypto"
import { homedir } from "node:os"
import { clearTimeout, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import {
  listWorkflowRegistryRequest,
} from "@arroba/kernel-client"
import { BoxRenderable, ScrollBoxRenderable, TextRenderable, type TextareaRenderable } from "@opentui/core"
import { useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect } from "solid-js"
import { reconcile } from "solid-js/store"

import type {
  BootstrapState,
  RuntimeSession,
  TerminalOutputRecord,
  TranscriptEntry,
} from "./cli-types.js"
import { createCliAgentPaneComposition } from "./cli-agent-pane-composition.js"
import { createCliAppState } from "./cli-app-state.js"
import { createCliAppCommandRoutingComposition } from "./cli-app-command-routing-composition.js"
import { createCliAppProcessRuntimeComposition } from "./cli-app-process-runtime-composition.js"
import {
  createCliAppWorkflowActionComposition,
  createCliAppWorkflowProjectionComposition,
} from "./cli-app-workflow-composition.js"
import { createCliOverlayInteractionComposition } from "./cli-overlay-interaction-composition.js"
import { createCliPrimaryTranscriptComposition } from "./cli-primary-transcript-composition.js"
import { createCliPromptSurfaceComposition } from "./cli-prompt-surface-composition.js"
import { createCliResponseShellComposition } from "./cli-response-shell-composition.js"
import { createCliRuntimeProjectionComposition } from "./cli-runtime-projection-composition.js"
import { createCliSessionLifecycleComposition } from "./cli-session-lifecycle-composition.js"
import { createCliTranscriptRuntimeComposition } from "./cli-transcript-runtime-composition.js"
import { createCliWaitingRoomComposition } from "./cli-waiting-room-composition.js"
import { createCliClosingStateController } from "./cli-closing-state-controller.js"
import {
  COMMAND_CENTER_OVERLAY_FOOTPRINT,
  RENDER_SCHEDULER_RENDERABLES_PER_FLUSH,
  RENDER_SCHEDULER_TREES_PER_FLUSH,
} from "./cli-runtime-tuning.js"
import { CliAppWorkspaceView } from "./cli-app-workspace-view.js"
import { createAgentFocusTransitionController } from "./agent-focus-transition-controller.js"
import {
  createCliRendererFocusController,
} from "./cli-renderer-focus-controller.js"
import { createCommandCenterLayoutController } from "./command-center-layout-controller.js"
import { createCommandCenterController } from "./command-center-controller.js"
import type { CommandCenterWorkflowRegistryEntry } from "./command-center-context.js"
import { renderCommandCenterOverlay } from "./command-center-renderer.js"
import { workflowRegistrySuggestionEntriesFromResponse } from "./workflow-registry-command-center-entries.js"
import { commandTreeFromTerminalCommandCatalog } from "./terminal-command-catalog.js"
import { createAgentPaneRuntimeResetController } from "./agent-pane-runtime-reset-controller.js"
import { createAgentPaneRuntimeStoreController } from "./agent-pane-runtime-store-controller.js"
import { createFooterFlashController } from "./footer-flash-controller.js"
import { createHistoryLoadingRenderController } from "./history-loading-render-controller.js"
import { createHistoryScrollRestoreController } from "./history-scroll-restore-controller.js"
import { renderHistoryLoadingIndicator as renderHistoryLoadingIndicatorView } from "./history-loading-renderer.js"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import {
  DEBUG_LOGS_ENABLED,
  formatError,
  getLogger,
} from "./cli-runtime-singletons.js"
import {
  createCliUiBatchController,
} from "./cli-ui-batch-controller.js"
import { createPromptAttachmentIntakeController } from "./prompt-attachment-intake-controller.js"
import { createPromptSubmissionAgentStateController } from "./prompt-submission-agent-state-controller.js"
import {
  createPromptTextController,
} from "./prompt-text-controller.js"
import { createPromptStopController } from "./prompt-stop-controller.js"
import { createPrimaryTranscriptRuntimeStoreController } from "./primary-transcript-runtime-store-controller.js"
import {
  cancelActivePrompt,
  cancelQueuedPrompt,
  steerQueuedPrompt,
} from "./prompt-runtime-api.js"
import {
  syncQueuedPromptEntriesByAgent,
  syncQueuedPromptEntriesForAgent,
} from "./queued-prompt-transcript.js"
import {
  queuedPromptStripItemsForAgent,
  queuedPromptStripItemToTranscriptEntry,
  type QueuedPromptStripItem,
} from "./queued-prompt-strip-state.js"
import {
  type BackendProviderId,
  normalizeBackendProviderId,
} from "./provider-catalog.js"
import {
  getProviderCatalog,
  getProviderRun,
  tryGetProviderRun,
} from "./provider-api.js"
import { createPromptInputRefController } from "./prompt-input-ref-controller.js"
import { createResponsePaneRenderRefStoreController } from "./response-pane-render-ref-store-controller.js"
import { createResponsePaneRenderScheduleController } from "./response-pane-render-schedule-controller.js"
import {
  extractPromptHistoryEntries,
} from "./prompt-history.js"
import {
  DEFAULT_CONNECTED_STATUS,
  getSessionStatusLabel,
} from "./runtime.js"
import {
  type SessionChromeUpdateController,
} from "./session-chrome-update-controller.js"
import {
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
  focusedAgentIdForSession,
} from "./session-state.js"
import { createSessionStateApplyController } from "./session-state-apply-controller.js"
import { resolveTerminalRecordAgentId as resolveTerminalRecordAgentIdFromState } from "./terminal-record-agent-resolver.js"
import { createTranscriptScrollboxRefController } from "./transcript-scrollbox-ref-controller.js"
import { createTranscriptRenderDeferralController } from "./transcript-render-deferral-controller.js"
import {
  type ToolTranscriptUpdate,
} from "./transcript.js"
import {
  decideBootstrapAction,
  formatSessionList,
  selectAttachableSession,
  type SessionListEntry,
} from "./sessions.js"
import {
  aliasSession,
  archiveSessionById,
  attachToSession,
  deleteSessionByRef,
  detachSessionAttachment,
  getSessionState,
  listSessions,
  resolveSession,
} from "./session-api.js"
import {
  catchUpAttachedSession,
  pollRuntimeNotices,
  pumpTerminalOutput,
  resizeSessionTerminal as maybeResize,
} from "./session-runtime-api.js"
import {
  createSplitPaneFooterRenderState,
} from "./split-pane-footer-renderer.js"
import {
  createStatusIndicatorRenderState,
} from "./status-indicator-renderer.js"
import { createRenderScheduler } from "./render-scheduler.js"
import { createTranscriptSyntaxStyle } from "./theme.js"
import {
  deriveWorkspaceShellContextForSession,
} from "./workspace-shell-controller.js"
import {
  getWorkspaceLiveSyncStatus,
} from "./workspace-link-api.js"
import {
  computeCurrentTurnId,
  computeNextTurnId,
} from "./transcript-preview.js"
import {
  type TranscriptEntryRenderable,
  type TranscriptSurfaceTone,
} from "./transcript-render.js"
import { createTranscriptSyntaxStyleController } from "./transcript-syntax-style-controller.js"
import { createTranscriptTurnStateController } from "./transcript-turn-state-controller.js"

export function ArrobaCliApp(props: { bootstrap: BootstrapState }) {
  const {
    client,
    options,
    supportsKernelEventStream,
    initialBinding,
    initialSession,
    initialEntries,
    initialPromptDraft,
    initialWorkspaceTarget,
    initialWorktreeTarget,
    preferencesState,
    setPreferencesState,
    themeRevision,
    setThemeRevision,
    maxAgentsPerScreen,
    sessionState,
    setSessionState,
    attachmentState,
    setAttachmentState,
    providerRunState,
    setProviderRunState,
    createdSessionState,
    setCreatedSessionState,
    availableSessions,
    setAvailableSessions,
    providerCatalogState,
    setProviderCatalogState,
    providerCommandCatalogState,
    setProviderCommandCatalogState,
    terminalCommandCatalogState,
    setTerminalCommandCatalogState,
    themeRegistryState,
    relayStatusState,
    setRelayStatusState,
    remoteMachinesState,
    setRemoteMachinesState,
    remoteKernelsState,
    setRemoteKernelsState,
    slicesState,
    setSlicesState,
    terminalsState,
    setTerminalsState,
    externalProviderSessionsState,
    setExternalProviderSessionsState,
    externalProviderSessionsPageState,
    setExternalProviderSessionsPageState,
    waitingRoomInventoryStatus,
    setWaitingRoomInventoryStatus,
    waitingRoomHiddenKernelController,
    waitingRoomCloudNotice,
    setWaitingRoomCloudNotice,
    terminalPairingOpen,
    setTerminalPairingOpen,
    terminalPairingState,
    setTerminalPairingState,
    terminalPairingQrLines,
    setTerminalPairingQrLines,
    sessionBrowserOpen,
    setSessionBrowserOpen,
    agentLocationLabel,
    sessionBrowserIndex,
    setSessionBrowserIndex,
    waitingRoomState,
    setWaitingRoomState,
    pendingWorkspaceTarget,
    setPendingWorkspaceTarget,
    pendingWorktreeTarget,
    setPendingWorktreeTarget,
    multiAgentResponseLayout,
    setMultiAgentResponseLayout,
    entries,
    setEntries,
    activeStatusLabel,
    setActiveStatusLabel,
    providerActivityLabel,
    setProviderActivityLabel,
    agentActivityLabels,
    setAgentActivityLabels,
    streamingAgentId,
    setStreamingAgentId,
    statusLine,
    setStatusLine,
    fatalError,
    setFatalError,
    submitting,
    setSubmitting,
    entryCounter,
    setEntryCounter,
    daemonDisconnected,
    setDaemonDisconnected,
    kernelConnected,
    setKernelConnected,
    nextHistoryCursor,
    setNextHistoryCursor,
    agentPanePreviews,
    setAgentPanePreviews,
    agentPaneEntries,
    setAgentPaneEntries,
    agentBusyLatches,
    setAgentBusyLatches,
    sessionHydrating,
    setSessionHydrating,
    loadingHistory,
    setLoadingHistory,
    workingAnimationFrame,
    setWorkingAnimationFrame,
    working,
    setWorking,
    footerFlash,
    setFooterFlash,
    pendingAttachments,
    setPendingAttachments,
    promptHistoryEntries,
    setPromptHistoryEntries,
    promptHistoryIndex,
    setPromptHistoryIndex,
    promptHistoryDraft,
    setPromptHistoryDraft,
    hotkeysOpen,
    setHotkeysOpen,
    expandedTurnIdsByAgent,
    setExpandedTurnIdsByAgent,
    workspaceScreenMode,
    setWorkspaceScreenMode,
    workspaceShellContext,
    setWorkspaceShellContext,
    workspaceShellEntries,
    setWorkspaceShellEntries,
    workspaceShellEntryCounter,
    setWorkspaceShellEntryCounter,
    workspaceLiveSyncStatus,
    setWorkspaceLiveSyncStatus,
    selectedWorkflowId,
    setSelectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
    selectedWorkflowComponent,
    setSelectedWorkflowComponent,
    workflowInspectorMode,
    setWorkflowInspectorMode,
    workflowNodeInstructionsEditor,
    setWorkflowNodeInstructionsEditor,
  } = createCliAppState({
    bootstrap: props.bootstrap,
    cwd: process.cwd(),
  })
  const appLogger = getLogger("cli.app", {
    session_id: initialBinding?.session.id ?? null,
    attachment_id: initialBinding?.attachment.id ?? null,
    client_id: options.clientId,
  })
  const renderer = useRenderer()
  const dimensions = useTerminalDimensions()
  const setCenterMode = (_mode: "transcript") => {}
  const setDirectoryTreeState = (_value: null) => {}
  const promptInputRefController = createPromptInputRefController<TextareaRenderable>()
  const transcriptScrollboxRefController = createTranscriptScrollboxRefController<ScrollBoxRenderable>()
  const responsePaneRenderRefStore = createResponsePaneRenderRefStoreController<
    BoxRenderable,
    ScrollBoxRenderable,
    TextRenderable
  >()
  const splitPaneFooterRenderState = createSplitPaneFooterRenderState()
  const interactionChoiceStore = createInteractionChoiceStoreController()
  const agentPaneRuntimeStore = createAgentPaneRuntimeStoreController<
    ScrollBoxRenderable,
    TranscriptEntryRenderable,
    BoxRenderable,
    ToolTranscriptUpdate
  >()
  const statusIndicatorRenderState = createStatusIndicatorRenderState()
  const closingStateController = createCliClosingStateController()
  const primaryTranscriptRuntimeStore = createPrimaryTranscriptRuntimeStoreController<
    TranscriptEntryRenderable,
    BoxRenderable,
    ToolTranscriptUpdate
  >({
    initialMountedTranscriptAgentId: initialBinding
      ? focusedAgentIdForSession(initialSession)
      : null,
  })
  const transcriptSyntaxStyleController = createTranscriptSyntaxStyleController({
    createStyle: createTranscriptSyntaxStyle,
  })
  const historyScrollRestoreController = createHistoryScrollRestoreController({
    scheduleTimer: (callback, delayMs) => {
      startTimeout(callback, delayMs)
    },
    getScrollbox: transcriptScrollboxRefController.current,
    setLastScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
  })
  let sessionChromeUpdateController: SessionChromeUpdateController
  const uiBatchController = createCliUiBatchController({
    batch,
    flushDeferredUpdates: () => {
      transcriptRenderDeferralController.flush()
      sessionChromeUpdateController.flushDeferred()
    },
  })
  const agentFocusTransitionController = createAgentFocusTransitionController()
  const transcriptTurnStateController = createTranscriptTurnStateController({
    initialCurrentTurnId: computeCurrentTurnId(initialEntries),
    initialNextTurnId: computeNextTurnId(initialEntries),
  })
  const promptSubmissionAgentStateController = createPromptSubmissionAgentStateController()
  const promptTextController = createPromptTextController({
    initialText: initialPromptDraft,
    getPromptInput: promptInputRefController.currentOrNull,
    refreshHighlights: () => refreshPromptAttachmentHighlights(),
  })
  const renderScheduler = createRenderScheduler({
    schedule: (callback) => startTimeout(callback, 0),
    clearSchedule: clearTimeout,
    maxTreesPerFlush: RENDER_SCHEDULER_TREES_PER_FLUSH,
    maxRenderablesPerFlush: RENDER_SCHEDULER_RENDERABLES_PER_FLUSH,
    requestRootRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
  })
  const transcriptRenderDeferralController = createTranscriptRenderDeferralController({
    isBatched: uiBatchController.isBatched,
    getRenderable: transcriptScrollboxRefController.current,
    requestRender: (renderable) => {
      renderScheduler.requestRenderable(renderable)
    },
  })

  const {
    isAttached,
    focusedAgentId,
    multiAgentMode,
    workflowScreenShowing,
    splitAgentResponseMode,
    activeInteractionForAgent,
    focusedAgentInteraction,
    workflowPromptState,
    responsePaneSelection,
    responsePaneAgentSignature,
    responsePrimaryAgent,
    responseVisibleAgents,
    visibleTranscriptAgentId,
    responsePaneRows,
    primaryTranscriptSurfaceTone,
    auxiliaryTranscriptSurfaceTone,
    agentActivityLabel,
    focusedAgent,
    focusedBackendProvider,
    focusedProviderRun,
    resolveSessionAgent,
    agentBusyLatch,
    anyPromptWork,
    hasPromptWorkByAgent,
    focusedQueueDepth,
    focusedActivePrompt,
    focusedActivityLabel,
    markAgentBusy,
    clearAgentBusy,
    focusedAgentBusy,
    allAgentsBusyState,
    setAgentActivityLabel,
    transcriptEntryProjectionController,
    visibleTranscriptEntries,
    connectedClientCount,
    activePrompt,
    focusedStatusBadge,
    runtimeDebugLogger,
    logProviderRunDebug,
    logViewDebug,
    logVisibleTranscriptOutput,
    logFocusedBadgeChange,
  } = createCliRuntimeProjectionComposition({
    appLogger,
    debugLogsEnabled: DEBUG_LOGS_ENABLED,
    attachmentState,
    sessionState,
    workspaceScreenMode,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    workflowScreenActive: () => workflowActions.workflowScreenActive(),
    selectedWorkflowId,
    selectedWorkflowNodeId,
    providerRunState,
    primaryTranscriptRuntimeStore,
    agentPaneRuntimeStore,
    agentPanePreviews,
    agentActivityLabels,
    setAgentActivityLabels,
    agentBusyLatches,
    setAgentBusyLatches,
    submitting,
    promptSubmissionAgentStateController,
    streamingAgentId,
    entries: () => entries,
    daemonDisconnected,
    transcriptScrollboxRefController,
  })
  createEffect(() => {
    if (!isAttached()) {
      return
    }
    const session = sessionState()
    setWorkspaceShellContext((previous) =>
      deriveWorkspaceShellContextForSession(previous, session, attachmentState()?.id))
  })
  const historyLoadingRenderController = createHistoryLoadingRenderController({
    renderer,
    loading: loadingHistory,
    renderIndicator: renderHistoryLoadingIndicatorView,
  })
  const renderHistoryLoadingIndicator = historyLoadingRenderController.render
  const responsePaneRenderScheduleController = createResponsePaneRenderScheduleController({
    responseLayoutBox: responsePaneRenderRefStore.getLayoutBox,
    historyLoadingBox: historyLoadingRenderController.getBox,
    requestTree: (renderable) => renderScheduler.requestTree(renderable),
    requestRoot: () => renderScheduler.requestRoot(),
  })
  const scheduleResponsePaneRepaint = responsePaneRenderScheduleController.scheduleRepaint
  const {
    workflowInspector,
    bindWorkflowNodeInstructionsEditor,
  } = createCliAppWorkflowProjectionComposition({
    sessionState,
    selectedWorkflowId,
    selectedWorkflowNodeId,
    selectedWorkflowComponent,
    workflowInspectorMode,
    workflowNodeInstructionsEditor,
    agentPaneEntries,
    setSelectedWorkflowId,
    setSelectedWorkflowNodeId,
  })
  const promptStopController = createPromptStopController({
    getAttachment: attachmentState,
    getActivePrompt: activePrompt,
    getSessionId: () => sessionState().id,
    getFallbackStreamingAgentId: streamingAgentId,
    cancelActivePrompt: (sessionId, attachmentId) => cancelActivePrompt(client, sessionId, attachmentId),
    onCancellationRequested: (targetAgentId) => {
      appLogger?.info("requested active prompt cancellation")
      setStatusLine("Cancellation requested.")
      setStreamingAgentId(targetAgentId)
      setWorking(true)
      updateSessionChrome()
    },
    onCancellationFailed: (error) => {
      appLogger?.error("active prompt cancellation failed", {
        error: formatError(error),
      })
      setFatalError(formatError(error))
      updateSessionChrome()
    },
  })
  const resolveTerminalRecordAgentId = (record: TerminalOutputRecord) => {
    return resolveTerminalRecordAgentIdFromState({
      record,
    })
  }
  const rendererFocusController = createCliRendererFocusController(renderer)
  const describeRenderableDebug = rendererFocusController.describe
  const currentFocusedRenderable = rendererFocusController.current
  const {
    activateWaitingRoom,
    applyModelSelection,
    applyProviderCatalogChanged,
    applyProviderSelection,
    applyRelayStatusChanged,
    applyRemoteMachinesChanged,
    applySlicesChanged,
    applyVariantSelection,
    applyWaitingRoomRowsChanged,
    applyWaitingRoomSessionLifecycleAction,
    connectDetachedKernelFromWaitingRoom,
    currentModelId,
    currentProviderSelection,
    currentVariantId,
    promptMetaParts,
    promptUsageMeta,
    reconcileWaitingRoom,
    refreshWaitingRoomData,
    refreshWaitingRoomDataNow,
    startSessionFromWaitingRoomDefaults,
    waitingRoomTargets,
  } = createCliWaitingRoomComposition({
    client,
    options,
    appLogger,
    formatError,
    isAttached,
    kernelConnected,
    waitingRoomState,
    setWaitingRoomState,
    availableSessions,
    setAvailableSessions,
    providerCatalogState,
    setProviderCatalogState,
    providerCommandCatalogState,
    setProviderCommandCatalogState,
    themeRegistryState,
    waitingRoomCloudNotice,
    waitingRoomInventoryStatus,
    setWaitingRoomInventoryStatus,
    waitingRoomHiddenKernelController,
    relayStatusState,
    setRelayStatusState,
    remoteMachinesState,
    setRemoteMachinesState,
    remoteKernelsState,
    setRemoteKernelsState,
    terminalsState,
    setTerminalsState,
    externalProviderSessionsState,
    setExternalProviderSessionsState,
    externalProviderSessionsPageState,
    setExternalProviderSessionsPageState,
    slicesState,
    setSlicesState,
    pendingWorkspaceTarget,
    pendingWorktreeTarget,
    preferencesState,
    setPreferencesState,
    setThemeRevision,
    resetTranscriptSyntax: () => {
      transcriptSyntaxStyleController.reset()
    },
    applyResponseLayout: () => applyResponseLayout(),
    renderCommandCenter: () => renderCommandCenter(),
    rebuildTranscript: () => rebuildTranscript(),
    updateSessionChrome: () => updateSessionChrome(),
    syncCommandCenter: (text?: string) => syncCommandCenter(text),
    handleCloudCommand: (command) => handleCloudCommand(command),
    setPromptText: (text) => setPromptText(text),
    focusPrompt: () => {
      promptInputRefController.focus()
    },
    openTerminalPairingDialog: () => openTerminalPairingDialog(),
    openSessionBrowserDialog: () => openSessionBrowserDialog(),
    closeSessionBrowserDialog: () => closeSessionBrowserDialog(),
    attachBinding: (session, createdSession, launch) => attachBinding(session, createdSession, launch),
    flashFooter: (message, tone) => flashFooter(message, tone),
    setKernelConnected,
    setDaemonDisconnected,
    sessionBrowserOpen,
    focusedProviderRun,
    focusedAgent,
    focusedAgentId,
    providerRunState,
    sessionState,
    applySessionState: (session) => applySessionState(session),
    setProviderRunState,
    appendNotice: (text) => appendNotice(text),
  })
  const commandCenterLayoutController = createCommandCenterLayoutController({
    terminalHeight: () => dimensions().height,
    promptHeight: () => promptInputRefController.height(1),
  })
  const commandCenterVisibleRowCount = commandCenterLayoutController.visibleRowCount
  let workflowRegistrySuggestionEntries: CommandCenterWorkflowRegistryEntry[] = []
  let workflowRegistrySuggestionSessionId: string | null = null
  let workflowRegistrySuggestionFetchedAtMs = 0
  let workflowRegistrySuggestionFetchInFlight = false
  let resyncWorkflowRegistrySuggestions: (() => void) | null = null
  const invalidateWorkflowRegistrySuggestions = () => {
    workflowRegistrySuggestionEntries = []
    workflowRegistrySuggestionSessionId = null
    workflowRegistrySuggestionFetchedAtMs = 0
  }
  const refreshWorkflowRegistrySuggestions = (input: string) => {
    if (!shouldRefreshWorkflowRegistrySuggestions(input)) {
      return
    }
    const sessionId = sessionState().id
    const nowMs = Date.now()
    if (
      workflowRegistrySuggestionFetchInFlight
      || (workflowRegistrySuggestionSessionId === sessionId && nowMs - workflowRegistrySuggestionFetchedAtMs < 5000)
    ) {
      return
    }
    workflowRegistrySuggestionFetchInFlight = true
    void client.send<Record<string, unknown>>(listWorkflowRegistryRequest(sessionId))
      .then((response) => {
        workflowRegistrySuggestionEntries = workflowRegistrySuggestionEntriesFromResponse(response)
        workflowRegistrySuggestionSessionId = sessionId
        workflowRegistrySuggestionFetchedAtMs = Date.now()
        resyncWorkflowRegistrySuggestions?.()
      })
      .catch((error) => {
        getLogger("workflow-registry-suggestions")?.debug("workflow registry suggestion refresh failed", { error: formatError(error) })
      })
      .finally(() => {
        workflowRegistrySuggestionFetchInFlight = false
      })
  }
  const commandCenterController = createCommandCenterController<BoxRenderable>({
    getCommandTree: () => commandTreeFromTerminalCommandCatalog(terminalCommandCatalogState()),
    getProviderCatalog: providerCatalogState,
    getProviderCommandCatalogs: providerCommandCatalogState,
    getCurrentProvider: () => normalizeBackendProviderId(currentProviderSelection().provider),
    getFocusedProvider: focusedBackendProvider,
    getCurrentModel: currentModelId,
    getCurrentVariant: currentVariantId,
    getWorkflowRegistryEntries: () => workflowRegistrySuggestionEntries,
    refreshWorkflowRegistryEntries: refreshWorkflowRegistrySuggestions,
    getPromptText: promptTextController.currentText,
    replacePromptText: promptTextController.setText,
    executeCommand: async (command) => {
      await executeCommandCenterCommand(command)
      if (shouldInvalidateWorkflowRegistrySuggestions(command)) {
        invalidateWorkflowRegistrySuggestions()
      }
    },
    onCommandError: (error) => {
      flashFooter(formatError(error), "error")
    },
    render: (state, box) => {
      renderCommandCenterOverlay({
        box,
        renderer,
        open: state.open,
        items: state.items,
        selectedIndex: state.selectedIndex,
        visibleRowCount: commandCenterVisibleRowCount(),
        promptHeight: promptInputRefController.height(1),
        overlayFootprint: COMMAND_CENTER_OVERLAY_FOOTPRINT,
      })
    },
  })
  const syncCommandCenter = commandCenterController.sync
  const commandCenterOpen = commandCenterController.open
  const clearCommandCenter = commandCenterController.clear
  const handleCommandCenterKey = commandCenterController.handleKey
  const selectCommandCenterFromSubmit = commandCenterController.selectFromSubmit
  const renderCommandCenter = commandCenterController.render
  resyncWorkflowRegistrySuggestions = () => {
    if (commandCenterOpen() && shouldRefreshWorkflowRegistrySuggestions(promptTextController.currentText())) {
      syncCommandCenter(promptTextController.currentText())
    }
  }
  const {
    addPendingPromptAttachments,
    appendPromptEchoToSharedHistory,
    beginSubmittedPromptUi,
    clearPendingPromptAttachments,
    clearPendingPromptDraftPersist,
    flushPendingPromptDraftPersist,
    footerHint,
    handlePromptContentChange,
    navigatePromptHistoryInput,
    persistablePromptDraft,
    persistSessionPromptState,
    promptAreaBackground,
    promptHistoryHydrationController,
    promptInputHistoryRefreshController,
    promptInputMaxHeight,
    promptPlaceholder,
    recordPromptAreaHistoryEntry,
    refreshPromptAttachmentHighlights,
    removeLastPendingPromptAttachment,
    removePromptAttachmentsForEdit,
    restoreFailedPromptUi,
    retainPromptFocus,
    scheduleSharedPromptInputHistoryRefresh,
    sessionStatusMode,
    setPromptText,
    syncPromptPlaceholder,
    syncPromptTextSnapshot,
  } = createCliPromptSurfaceComposition({
    client,
    appLogger,
    formatError,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    daemonDisconnected,
    working,
    anyPromptWork,
    submitting,
    focusedQueueDepth,
    fatalError,
    focusedActivePrompt,
    statusLine,
    isAttached,
    workflowScreenShowing,
    workflowPromptState,
    themeRevision,
    preferencesState,
    setPreferencesState,
    setPromptHistoryEntries,
    setPromptHistoryIndex,
    setPromptHistoryDraft,
    promptTextController,
    attachmentState,
    promptHistoryEntries,
    promptHistoryIndex,
    promptHistoryDraft,
    promptInputRefController,
    pendingAttachments,
    setPendingAttachments,
    terminalHeight: () => dimensions().height,
    requestRender: () => (renderer as { requestRender?: () => void }).requestRender?.(),
    updateSessionChrome: () => updateSessionChrome(),
    syncCommandCenter,
    clearCommandCenter,
    attachPromptFiles: (files, insertAt) => attachPromptFiles(files, insertAt),
    getCwd: () => process.cwd(),
    flashFooter: (message, tone) => flashFooter(message, tone),
  })
  const {
    assignDialogOverlayBox,
    closeActiveDialogOverlay,
    closeHotkeys,
    closeSessionBrowserDialog,
    closeTerminalPairingDialog,
    copyPromptSelection,
    dialogOverlayOpen,
    handleHotkeysToggleShortcut,
    handlePromptSelectionSurfaceMouseUp,
    handleSessionBrowserKey,
    openHotkeys,
    openSessionBrowserDialog,
    openTerminalPairingDialog,
    renderHotkeysOverlay,
  } = createCliOverlayInteractionComposition({
    client,
    renderer,
    dimensions,
    appLogger,
    formatError,
    debugLogsEnabled: DEBUG_LOGS_ENABLED,
    isAttached,
    availableSessions,
    sessionBrowserIndex,
    setSessionBrowserIndex,
    currentFocusedRenderable,
    promptInputRefController,
    describeRenderableDebug,
    scheduleTimer: startTimeout,
    hotkeysOpen,
    setHotkeysOpen,
    terminalPairingOpen,
    setTerminalPairingOpen,
    terminalPairingState,
    setTerminalPairingState,
    terminalPairingQrLines,
    setTerminalPairingQrLines,
    sessionBrowserOpen,
    setSessionBrowserOpen,
    waitingRoomState,
    providerCatalogState,
    options,
    flashFooter: (message, tone) => flashFooter(message, tone),
    attachBinding: (session, createNew, launch) => attachBinding(session, createNew, launch),
    applyWaitingRoomSessionLifecycleAction,
    retainPromptFocus,
  })
  const {
    turnCompletionController,
    cancelPendingTurnCompletion,
    recordTurnActivity,
    expandedTurnIdsForAgent,
    setExpandedTurnState,
    applyExpandedTurns,
    toggleTurn,
    toggleBlob,
    appendEntry,
    appendUserPrompt,
    appendSteeredPrompt,
    appendNotice,
    appendCloudNotice,
    appendProviderError,
    clearLocalBusyStateForAuthoritativeIdle,
    applyProviderActivity,
    markAssistantMessageCompleted,
    syncVisibleActivityLabel,
    appendProviderChunk,
    appendToolUpdate,
    queueTerminalOutputRecords,
    clearTerminalOutputRecordTimer,
    setKernelTerminalOutputRecordProcessor,
  } = createCliTranscriptRuntimeComposition({
    batchUpdate: batch,
    client,
    formatError,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    runUiBatch: (callback) => runUiBatch(callback),
    entries: () => entries,
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    entryCounter,
    setEntryCounter,
    sessionState,
    activePrompt,
    statusLine,
    setStatusLine,
    setWorking,
    setSubmitting,
    setStreamingAgentId,
    setAgentActivityLabels,
    setAgentBusyLatches,
    setProviderActivityLabel,
    setActiveStatusLabel,
    promptSubmissionAgentStateController,
    promptStopController,
    appendPromptEchoToSharedHistory,
    focusedAgentId,
    visibleTranscriptAgentId,
    responsePrimaryAgent,
    splitAgentResponseMode,
    isAttached,
    currentAgentPaneEntries: (agentId) => currentAgentPaneEntries(agentId),
    appendTranscriptEntryToAgentPane: (agentId, entry, turnIds) => {
      appendTranscriptEntryToAgentPane(agentId, entry, turnIds)
    },
    transcriptEntryProjectionController,
    transcriptTurnStateController,
    expandedTurnIdsByAgent,
    setExpandedTurnIdsByAgent,
    persistVisibleTranscriptEntries: (nextEntries) => {
      persistVisibleTranscriptEntries(nextEntries)
    },
    reconcileMountedTranscript: (currentEntries, nextEntries) => {
      reconcileMountedTranscript(currentEntries, nextEntries)
    },
    retainPromptFocus,
    transcriptScrollboxRefController,
    historyScrollRestoreController,
    primaryTranscriptRuntimeStore,
    clearAgentBusy,
    markAgentBusy,
    setWaitingRoomCloudNotice,
    renderSessionChromeBoundary: () => renderSessionChromeBoundary(),
    syncVisibleTranscriptPreview: () => syncVisibleTranscriptPreview(),
    updateSessionChrome: () => updateSessionChrome(),
    rebuildTranscript: () => rebuildTranscript(),
    focusedActivityLabel,
    logVisibleTranscriptOutput,
    updateTranscriptEntry: (entryId, text, sourceText) => {
      updateTranscriptEntry(entryId, text, sourceText)
    },
    setAgentTranscriptEntries: (agentId, nextEntries) => {
      setAgentTranscriptEntries(agentId, nextEntries)
    },
    DEFAULT_CONNECTED_STATUS,
  })

  const trackAgentFocusTransition = <T,>(operation: () => Promise<T>): Promise<T> =>
    agentFocusTransitionController.track(operation)

  const waitForPendingAgentFocusTransition = (): Promise<void> =>
    agentFocusTransitionController.wait()

  const footerFlashController = createFooterFlashController({
    delayMs: 10_000,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    setFooterFlash,
    onFooterFlashChange: () => updateSessionChrome(),
  })
  const flashFooter = footerFlashController.flash
  let syncQueuedPromptsForSession = (_session: RuntimeSession) => {}

  const promptAttachmentIntakeController = createPromptAttachmentIntakeController({
    client,
    cwd: () => process.cwd(),
    sessionState,
    attachmentState,
    promptInsertOffset: promptTextController.cursorOffset,
    addPendingPromptAttachments,
    clearPendingPromptAttachments,
    flashFooter,
  })
  const attachPromptFiles = promptAttachmentIntakeController.attachFiles
  const handleAttachmentCommand = promptAttachmentIntakeController.handleCommand

  const sessionStateApplyController = createSessionStateApplyController({
    getSession: sessionState,
    setSession: setSessionState,
    getFocusedAgentId: focusedAgentId,
    getCurrentResponseLayout: multiAgentResponseLayout,
    getLayoutPreference: () => preferencesState().ui?.multiAgentResponseLayout,
    setResponseLayout: setMultiAgentResponseLayout,
    getWorking: working,
    setWorking,
    getSubmitting: submitting,
    setSubmitting,
    clearSubmittingAgentId: promptSubmissionAgentStateController.clearSubmittingAgentId,
    getAgentBusyLatches: agentBusyLatches,
    getAgentActivityLabels: agentActivityLabels,
    setAgentActivityLabels,
    clearAgentBusy,
    getStreamingAgentId: streamingAgentId,
    setStreamingAgentId,
    getProviderActivityLabel: providerActivityLabel,
    setProviderActivityLabel,
    getActiveStatusLabel: activeStatusLabel,
    setActiveStatusLabel,
    getStatusLine: statusLine,
    setStatusLine,
    clearActiveToolLabels: primaryTranscriptRuntimeStore.clearActiveToolLabels,
    turnCompletion: turnCompletionController,
    cancelPendingTurnCompletion,
    promptStop: promptStopController,
    syncQueuedPromptEntries: (session) => syncQueuedPromptsForSession(session),
    syncVisibleActivityLabel: () => syncVisibleActivityLabel(),
    updateSessionChrome: () => updateSessionChrome(),
    refreshSplitPaneFocusRepaint: () => refreshSplitPaneFocusRepaint(),
  })
  const applySessionState = sessionStateApplyController.apply

  const runUiBatch = uiBatchController.run

  const {
    renderSplitPaneFooters,
    renderAgentInteractions,
    assignPromptMetaRef,
    requestTranscriptRender,
    setHistoryLoadingState,
    setSessionHydratingState,
    applyResponseLayout,
    refreshSplitPaneFocusRepaint,
    renderSessionChromeBoundary,
    updateSessionChrome,
    sessionChromeUpdateController: responseShellSessionChromeUpdateController,
    assignStatusIndicatorBox,
    assignFooterSummaryBox,
  } = createCliResponseShellComposition({
    renderer,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    uiBatchController,
    splitPaneFooterRenderState,
    statusIndicatorRenderState,
    responsePaneRenderRefStore,
    transcriptScrollboxRefController,
    historyLoadingRenderController,
    scheduleResponsePaneRepaint,
    renderHistoryLoadingIndicator,
    transcriptEntryProjectionController,
    transcriptRenderDeferralController,
    isAttached,
    workflowScreenActive: () => workflowActions.workflowScreenActive(),
    maxAgentsPerScreen,
    responseVisibleAgents,
    focusedAgentId,
    providerRunState,
    currentProviderSelection,
    agentActivityLabels,
    hasPromptWorkByAgent,
    streamingAgentId,
    agentBusyLatch,
    agentBusyLatches,
    sessionState,
    workspaceLiveSyncStatus,
    agentLocationLabel,
    workingAnimationFrame,
    activeInteractionForAgent,
    queuedPromptStripItemsForAgent: (agentId) => queuedPromptStripItemsForAgent(
      sessionState(),
      agentId ? (agentPaneEntries()[agentId] ?? []) : [],
      agentId,
    ),
    onQueuedPromptAction: handleQueuedPromptStripAction,
    interactionChoiceStore,
    promptUsageMeta,
    sessionHydrating,
    setSessionHydrating,
    setLoadingHistory,
    rebuildTranscript: () => rebuildTranscript(),
    focusedStatusBadge,
    runtimeDebugLogger,
    logFocusedBadgeChange,
    splitAgentResponseMode,
    responsePaneRows,
    responsePaneSelection,
    workspaceScreenMode,
    multiAgentResponseLayout,
    terminalWidth: () => dimensions().width,
    responsePaneAgentSignature,
    clearAuxiliaryAgentPane: (agentId) => clearAuxiliaryAgentPane(agentId),
    unregisterAgentScrollbox: agentPaneRuntimeStore.unregisterScrollbox,
    getCurrentAuxiliaryAgentId: agentPaneRuntimeStore.getCurrentAuxiliaryAgentId,
    setCurrentAuxiliaryAgentId: agentPaneRuntimeStore.setCurrentAuxiliaryAgentId,
    registerAgentScrollbox: agentPaneRuntimeStore.registerScrollbox,
    rebuildAuxiliaryAgentPane: (agentId) => rebuildAuxiliaryAgentPane(agentId),
    primaryTranscriptRuntimeStore,
    agentPaneEntries,
    replaceTranscriptEntries: (nextEntries, agentId) => replaceTranscriptEntries(nextEntries, agentId),
    logViewDebug,
    promptSubmissionAgentStateController,
    setAgentBusyLatches,
    providerRunStateSignal: providerRunState,
    working,
    activeStatusLabel,
    providerActivityLabel,
    syncPromptPlaceholder,
    fatalError,
    submitting,
    footerHint,
    connectedClientCount,
    multiAgentMode,
    sessionStatusMode,
    footerFlash,
    promptMetaParts,
  })
  sessionChromeUpdateController = responseShellSessionChromeUpdateController

  const {
    clearAllAuxiliaryAgentPanes,
    clearAuxiliaryAgentPane,
    rebuildAuxiliaryAgentPane,
    persistVisibleTranscriptEntries,
    setAgentPanePreview,
    setAgentTranscriptEntries,
    currentAgentPaneEntries,
    hasTrailingUserPrompt,
    toggleAuxiliaryPaneTurn,
    toggleAuxiliaryPaneBlob,
    syncVisibleTranscriptPreview,
    appendAgentPanePreview,
    appendTranscriptEntryToAgentPane,
    appendProviderChunkToAgentPane,
    appendToolUpdateToAgentPane,
    refreshAgentPanes,
    shouldRefreshAgentPanesForSessionChange,
  } = createCliAgentPaneComposition({
    client,
    renderer,
    isAttached,
    visibleTranscriptAgentId,
    visibleTranscriptEntries: transcriptEntryProjectionController.renderableEntries,
    agentPaneEntries,
    setAgentPaneEntries,
    setAgentPanePreviews,
    setExpandedTurnIdsByAgent,
    setNextHistoryCursor,
    sessionState,
    focusedAgentId,
    maxAgentsPerScreen,
    splitAgentResponseMode,
    responsePrimaryAgent,
    expandedTurnIdsByAgent,
    expandedTurnIdsForAgent,
    setExpandedTurnState,
    applyExpandedTurns,
    retainPromptFocus,
    formatError,
    agentPaneRuntimeStore,
    transcriptSyntaxStyleController,
    auxiliaryTranscriptSurfaceTone,
    onQueuedPromptAction: handleQueuedPromptAction,
    renderScheduler,
    primaryTranscriptRuntimeStore,
    replaceTranscriptEntries: (nextEntries, agentId) => replaceTranscriptEntries(nextEntries, agentId),
    applyResponseLayout,
  })

  const {
    mountTranscriptEntry,
    reconcileMountedTranscript,
    updateTranscriptEntry,
    rebuildTranscript,
    replaceTranscriptEntries,
    primeAttachedSessionBinding,
    bumpHistoryLoadGeneration,
    transcriptHistoryAutoloadController,
  } = createCliPrimaryTranscriptComposition({
    client,
    bootstrap: props.bootstrap,
    renderer,
    appLogger,
    formatError,
    scheduleTimer: startTimeout,
    isAttached,
    sessionHydrating,
    loadingHistory,
    nextHistoryCursor,
    setNextHistoryCursor,
    entryCounter,
    setEntryCounter,
    setHistoryLoadingState,
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    setPromptHistoryEntries,
    setPromptHistoryIndex,
    setPromptHistoryDraft,
    setProviderCatalogState,
    setProviderCommandCatalogState,
    setTerminalCommandCatalogState,
    updateSessionChrome,
    flashFooter,
    attachmentState,
    sessionState,
    selectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
    selectedWorkflowComponent,
    setSelectedWorkflowComponent,
    setWorkflowInspectorMode,
    workflowScreenActive: () => workflowActions.workflowScreenActive(),
    workflowInspector,
    workspaceShellEntries,
    workspaceShellContext,
    waitingRoomState,
    availableSessions,
    providerCatalogState,
    waitingRoomCloudNotice,
    waitingRoomInventoryStatus,
    relayStatusState,
    remoteMachinesState,
    remoteKernelsState,
    terminalsState,
    externalProviderSessionsState,
    externalProviderSessionsPageState,
    waitingRoomTargets,
    themeRegistryState,
    transcriptScrollboxRefController,
    primaryTranscriptRuntimeStore,
    transcriptEntryProjectionController,
    visibleTranscriptAgentId,
    transcriptSyntaxStyleController,
    historyScrollRestoreController,
    transcriptTurnStateController,
    expandedTurnIdsForAgent,
    syncVisibleTranscriptPreview,
    toggleTurn,
    toggleBlob,
    onQueuedPromptAction: handleQueuedPromptAction,
    primaryTranscriptSurfaceTone,
    requestTranscriptRender,
    requestRootRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
    logViewDebug,
    promptHistoryHydrationController,
    splitAgentResponseMode,
    maxAgentsPerScreen,
    setAgentPaneEntries,
    setAgentPanePreview,
  })

  syncQueuedPromptsForSession = (session: RuntimeSession) => {
    let changed = false
    const byAgent = syncQueuedPromptEntriesByAgent(agentPaneEntries(), session)
    if (byAgent.changed) {
      changed = true
      setAgentPaneEntries(reconcile(byAgent.entriesByAgent))
      setAgentPanePreviews((current) => ({
        ...current,
        ...byAgent.previews,
      }))
    }

    const visibleAgentId = visibleTranscriptAgentId()
    if (!visibleAgentId) {
      if (changed) {
        renderAgentInteractions()
        applyResponseLayout()
      }
      return
    }
    const visibleSync = syncQueuedPromptEntriesForAgent(
      transcriptEntryProjectionController.renderableEntries(),
      session,
      visibleAgentId,
    )
    if (visibleSync.changed) {
      changed = true
      replaceTranscriptEntries(visibleSync.entries, visibleAgentId)
    }
    if (changed) {
      renderAgentInteractions()
      applyResponseLayout()
    }
  }

  function handleQueuedPromptAction(entry: TranscriptEntry, action: "steer" | "cancel") {
    const queuedPrompt = entry.queuedPrompt
    if (!queuedPrompt) {
      return
    }
    if (action === "steer" ? !queuedPrompt.canSteer : !queuedPrompt.canCancel) {
      flashFooter(
        action === "steer"
          ? queuedPrompt.steerDisabledReason ?? "Queued prompt steering is unavailable."
          : queuedPrompt.cancelDisabledReason ?? "Queued prompt cancellation is unavailable.",
        "info",
      )
      return
    }
    const attachment = attachmentState()
    if (!attachment) {
      flashFooter("No session attached.", "error")
      return
    }

    updateQueuedPromptEntryStatus(queuedPrompt.agentId, queuedPrompt.promptId, action === "steer" ? "steering" : "cancelling")
    void (async () => {
      try {
        const payload = action === "steer"
          ? await steerQueuedPrompt(client, sessionState().id, attachment.id, queuedPrompt.agentId, queuedPrompt.promptId)
          : await cancelQueuedPrompt(client, sessionState().id, attachment.id, queuedPrompt.agentId, queuedPrompt.promptId)
        if (action === "steer") {
          appendSteeredPrompt(payload.prompt.prompt, queuedPrompt.agentId, {
            promptId: payload.prompt.id,
            sourceAttachmentId: payload.prompt.source_attachment_id,
          })
        }
        applySessionState(payload.session)
        updateSessionChrome()
      } catch (error) {
        updateQueuedPromptEntryStatus(queuedPrompt.agentId, queuedPrompt.promptId, "queued")
        flashFooter(formatError(error), "error")
      }
    })()
  }

  function handleQueuedPromptStripAction(item: QueuedPromptStripItem, action: "steer" | "cancel") {
    handleQueuedPromptAction(queuedPromptStripItemToTranscriptEntry(item), action)
  }

  function handleQueuedPromptStripKey(event: {
    name?: string
    eventType?: string
    ctrl?: boolean
    meta?: boolean
    alt?: boolean
    shift?: boolean
    preventDefault?: () => void
    stopPropagation?: () => void
  }) {
    if (
      !isAttached()
      || commandCenterOpen()
      || event.eventType === "release"
      || event.ctrl
      || event.meta
      || event.shift
      || !event.alt
    ) {
      return false
    }
    const action = event.name === "s"
      ? "steer"
      : event.name === "c"
        ? "cancel"
        : null
    if (!action) {
      return false
    }
    const agentId = focusedAgentId()
    const item = queuedPromptStripItemsForAgent(
      sessionState(),
      agentId ? (agentPaneEntries()[agentId] ?? []) : [],
      agentId,
    )[0]
    if (!item) {
      return false
    }
    event.preventDefault?.()
    event.stopPropagation?.()
    handleQueuedPromptStripAction(item, action)
    return true
  }

  function updateQueuedPromptEntryStatus(
    agentId: string,
    promptId: string,
    status: "queued" | "steering" | "cancelling",
  ) {
    const updateEntries = (currentEntries: TranscriptEntry[]) => currentEntries.map((candidate) => {
      if (candidate.queuedPrompt?.agentId !== agentId || candidate.queuedPrompt.promptId !== promptId) {
        return candidate
      }
      return {
        ...candidate,
        queuedPrompt: {
          ...candidate.queuedPrompt,
          status,
          steerDisabled: candidate.queuedPrompt.steerDisabled,
          canSteer: false,
          canCancel: false,
          steerDisabledReason: "This prompt is no longer waiting in the queue.",
          cancelDisabledReason: "This prompt is no longer waiting in the queue.",
        },
      }
    })
    if (visibleTranscriptAgentId() === agentId) {
      replaceTranscriptEntries(
        updateEntries(transcriptEntryProjectionController.renderableEntries()),
        agentId,
      )
    }
    setAgentTranscriptEntries(agentId, updateEntries(currentAgentPaneEntries(agentId)))
    renderAgentInteractions()
    applyResponseLayout()
  }

  const workflowActions = createCliAppWorkflowActionComposition({
    client,
    bindWorkflowNodeInstructionsEditor,
    workflowNodeInstructionsEditor,
    setWorkflowNodeInstructionsEditor,
    workflowScreenShowing,
    setWorkspaceScreenMode,
    rebuildTranscript,
    scheduleTimer: startTimeout,
    focusPromptInput: () => {
      promptInputRefController.focus()
    },
    setWorkflowInspectorMode,
    setSelectedWorkflowId,
    isAttached,
    sessionState,
    applySessionState,
    selectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
    setSelectedWorkflowComponent,
    workspaceScreenMode,
    applyResponseLayout,
  })

  const agentPaneRuntimeResetController = createAgentPaneRuntimeResetController({
    clearRenderedPanes: clearAllAuxiliaryAgentPanes,
    clearCurrentAuxiliaryAgentIds: agentPaneRuntimeStore.clearCurrentAuxiliaryAgentIds,
  })
  const clearAgentPaneRuntime = agentPaneRuntimeResetController.reset

  let recordDaemonActivity: (activityType: string) => void = () => {}
  const {
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
  } = createCliSessionLifecycleComposition({
    client,
    options,
    appLogger,
    renderer,
    sleep,
    formatError,
    supportsKernelEventStream,
    closingStateController,
    isAttached,
    daemonDisconnected,
    attachmentState,
    sessionState,
    providerRunState,
    createdSessionState,
    waitingRoomState,
    preferencesState,
    connectedClientCount,
    persistablePromptDraft,
    syncPromptTextSnapshot,
    flushPendingPromptDraftPersist,
    persistSessionPromptState,
    applySessionState,
    refreshAgentPanes,
    refreshSplitPaneFocusRepaint,
    maybeResize: (sessionId) => maybeResize(client, sessionId),
    catchUpAttachedSession: (sessionId, attachmentId, session) =>
      catchUpAttachedSession(client, sessionId, attachmentId, session, appLogger),
    primeAttachedSessionBinding,
    clearLocalBusyStateForAuthoritativeIdle,
    recordDaemonActivity: (activityType) => recordDaemonActivity(activityType),
    currentModelId,
    currentVariantId,
    focusedAgentId,
    clearPendingPromptAttachments,
    clearActiveToolLabels: primaryTranscriptRuntimeStore.clearActiveToolLabels,
    clearAgentPaneRuntime,
    setDirectoryTreeState,
    replaceTranscriptEntries,
    applyResponseLayout,
    setWorkspaceScreenMode,
    resetPromptStop: () => {
      promptStopController.reset()
    },
    bumpHistoryLoadGeneration,
    reconcileWaitingRoom,
    refreshWaitingRoomData,
    requestRootRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
    clearPromptInput: () => {
      promptInputRefController.clear()
    },
    blurPromptInput: () => {
      promptInputRefController.blur()
    },
    focusPromptInput: () => {
      promptInputRefController.focus()
    },
    setMultiAgentResponseLayout,
    setAttachmentState,
    setProviderRunState,
    setCenterMode,
    setCreatedSessionState,
    setSessionState,
    setProviderActivityLabel,
    setActiveStatusLabel,
    setAgentPaneEntries,
    setAgentPanePreviews,
    setAgentActivityLabels,
    setStreamingAgentId,
    setSubmitting,
    setWorking,
    setFatalError,
    setDaemonDisconnected,
    setNextHistoryCursor,
    setSessionHydratingState,
    setHistoryLoadingState,
    setStatusLine,
    setProviderCatalogState,
    setTerminalCommandCatalogState,
    availableSessions,
    setAvailableSessions,
    scheduleShortViewportHistoryCheck: () => transcriptHistoryAutoloadController.scheduleShortViewportCheck(),
    updateSessionChrome,
    appendNotice,
    flashFooter,
    logProviderRunDebug,
  })

  const {
    cycleFocusedInteractionChoice,
    executeCommandCenterCommand,
    handleCloudCommand,
    handlePromptKeyDown,
    handleSigint,
    handleStdinData,
    requestPromptStop,
    submitFocusedInteractionChoice,
    submitPrompt,
    submitWorkspaceShellCommand,
  } = createCliAppCommandRoutingComposition({
    client,
    options,
    appLogger,
    formatError,
    preferencesState,
    setPreferencesState,
    initialWorkspaceTarget,
    initialWorktreeTarget,
    pendingWorkspaceTarget,
    pendingWorktreeTarget,
    setPendingWorkspaceTarget,
    setPendingWorktreeTarget,
    isAttached,
    sessionState,
    attachmentState,
    providerRunState,
    currentModelId,
    currentVariantId,
    focusedAgentId,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    flashFooter,
    appendNotice,
    appendCloudNotice,
    attachBinding,
    transitionToNoSession,
    applyProviderSelection,
    applyModelSelection,
    applyVariantSelection,
    refreshWaitingRoomData,
    setSlicesState,
    setMultiAgentResponseLayout,
    applyResponseLayout,
    applySessionState,
    refreshAgentPanes,
    setWorkspaceLiveSyncStatus,
    ...workflowActions,
    rebuildTranscript,
    requestRootRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
    scheduleTimer: startTimeout,
    logViewDebug,
    describeRenderableDebug,
    currentFocusedRenderable,
    trackAgentFocusTransition,
    setProviderRunState,
    resolveSessionAgent,
    selectedWorkflowId,
    setSelectedWorkflowId,
    setSelectedWorkflowNodeId,
    refreshSplitPaneFocusRepaint,
    recordPromptAreaHistoryEntry,
    promptTextController,
    setPromptHistoryIndex,
    setPromptHistoryDraft,
    clearCommandCenter,
    requestExit,
    requestWaitingRoom,
    promptStopController,
    handleAttachmentCommand,
    workspaceShellContext,
    setWorkspaceShellContext,
    workspaceShellEntryCounter,
    setWorkspaceShellEntryCounter,
    setWorkspaceShellEntries,
    workflowPromptState,
    workflowInspectorMode,
    setWorkflowInspectorMode,
    selectedWorkflowNodeId,
    selectedWorkflowComponent,
    pendingAttachments,
    beginSubmittedPromptUi,
    restoreFailedPromptUi,
    focusedBackendProvider,
    workflowScreenShowing,
    waitForPendingAgentFocusTransition,
    primaryTranscriptRuntimeStore,
    setProviderActivityLabel,
    setActiveStatusLabel,
    appendUserPrompt,
    setStreamingAgentId,
    setWorking,
    updateSessionChrome,
    promptSubmissionAgentStateController,
    clearAgentBusy,
    setSubmitting,
    setFatalError,
    setStatusLine,
    promptInputRefController,
    ensureBackgroundPollersStarted: () => ensureBackgroundPollersStarted(),
    workflowNodeInstructionsEditor,
    openWorkflowNodeInstructionsEditor: workflowActions.openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor: workflowActions.closeWorkflowNodeInstructionsEditor,
    focusedAgentInteraction,
    interactionChoiceStore,
    renderAgentInteractions,
    handleHotkeysToggleShortcut,
    dialogOverlayOpen,
    closeActiveDialogOverlay,
    activePrompt,
    handleCommandCenterKey,
    handleQueuedPromptKey: handleQueuedPromptStripKey,
    commandCenterOpen,
    promptHistoryIndex,
    promptHistoryDraft,
    navigatePromptHistoryInput,
    visibleTranscriptEntries,
    transcriptScrollboxRefController,
    commandCenterController,
    waitingRoomState,
    availableSessions,
    providerCatalogState,
    relayStatusState,
    remoteMachinesState,
    setRemoteMachinesState,
    remoteKernelsState,
    terminalsState,
    slicesState,
    themeRegistryState,
    reconcileWaitingRoom,
    setWaitingRoomState,
    applyWaitingRoomSessionLifecycleAction,
    activateWaitingRoom,
    startSessionFromWaitingRoomDefaults,
    handleSessionBrowserKey,
    toggleWorkspaceScreen: workflowActions.toggleWorkspaceScreen,
    cycleWorkflowCanvasNode: workflowActions.cycleWorkflowCanvasNode,
    copyPromptSelection,
    removePromptAttachmentsForEdit,
    removeLastPendingPromptAttachment,
  })

  const {
    recordDaemonActivity: runtimeRecordDaemonActivity,
    ensureBackgroundPollersStarted,
    processKernelTerminalOutputRecord: runtimeProcessKernelTerminalOutputRecord,
  } = createCliAppProcessRuntimeComposition({
    client,
    options,
    appLogger,
    formatError,
    flashFooter,
    handleSigint,
    handleStdinData,
    clearTerminalOutputRecordTimer,
    workspaceScreenMode,
    workflowScreenActive: workflowActions.workflowScreenActive,
    daemonDisconnected,
    statusLine,
    sessionState,
    focusedAgentId,
    agentActivityLabels,
    hasPromptWorkByAgent,
    streamingAgentId,
    agentBusyLatch,
    isAttached,
    waitingRoomState,
    setWaitingRoomState,
    availableSessions,
    providerCatalogState,
    waitingRoomCloudNotice,
    waitingRoomInventoryStatus,
    relayStatusState,
    remoteMachinesState,
    remoteKernelsState,
    terminalsState,
    externalProviderSessionsState,
    externalProviderSessionsPageState,
    slicesState,
    waitingRoomTargets,
    themeRegistryState,
    selectedWorkflowId,
    selectedWorkflowNodeId,
    workspaceShellContext,
    workspaceShellEntries,
    transcriptEntries: () => entries,
    agentPaneEntries,
    footerFlash,
    getInteractionChoiceSelection: interactionChoiceStore.getSelectedIndex,
    getInteractionCustomReply: interactionChoiceStore.getStoredCustomReply,
    isInteractionCustomEditing: interactionChoiceStore.isCustomEditing,
    setInteractionCustomReply: interactionChoiceStore.setCustomReply,
    setInteractionCustomEditing: interactionChoiceStore.setCustomEditing,
    kernelConnected,
    setWorkspaceScreenMode,
    rebuildTranscript,
    applyResponseLayout,
    showWorkflowScreen: workflowActions.showWorkflowScreen,
    submitWorkspaceShellCommand,
    attachmentState,
    setPromptText,
    submitPrompt,
    activateWaitingRoom,
    connectDetachedKernelFromWaitingRoom,
    refreshWaitingRoomData,
    submitFocusedInteractionChoice,
    cycleFocusedInteractionChoice,
    toggleTurn,
    toggleAgentPaneTurn: toggleAuxiliaryPaneTurn,
    toggleBlob,
    toggleAgentPaneBlob: toggleAuxiliaryPaneBlob,
    restoreTerminalAndExit,
    sleep,
    closingStateController,
    supportsKernelEventStream,
    resizeSession: (sessionId: string) => maybeResize(client, sessionId),
    setDaemonDisconnected,
    setStatusLine,
    updateSessionChrome,
    appendNotice,
    working,
    recoverProviderRun,
    recordTurnActivity,
    resolveTerminalRecordAgentId,
    setStreamingAgentId,
    markAgentBusy,
    splitAgentResponseMode,
    visibleTranscriptAgentId,
    hasTrailingUserPrompt,
    currentAgentPaneEntries,
    appendTranscriptEntryToAgentPane,
    appendProviderChunkToAgentPane,
    appendToolUpdateToAgentPane,
    setAgentActivityLabel,
    agentActivityLabel,
    setProviderActivityLabel,
    applyProviderActivity,
    syncVisibleActivityLabel,
    appendEntry,
    appendProviderChunk,
    appendToolUpdate,
    appendProviderError,
    syncVisibleTranscriptPreview,
    appendAgentPanePreview,
    markAssistantMessageCompleted,
    providerRunState,
    shouldRefreshAgentPanesForSessionChange,
    applySessionState,
    logProviderRunDebug,
    setProviderRunState,
    refreshAgentPanes,
    catchUpAttachedSession: (sessionId: string, attachmentId: string, session: RuntimeSession) =>
      catchUpAttachedSession(client, sessionId, attachmentId, session, appLogger),
    getSessionState: (sessionId: string) => getSessionState(client, sessionId),
    getWorkspaceLiveSyncStatus: (sessionId: string) => getWorkspaceLiveSyncStatus(client, sessionId),
    setWorkspaceLiveSyncStatus,
    tryGetProviderRun: (providerRunId: string) => tryGetProviderRun(client, providerRunId, appLogger),
    clearLocalBusyStateForAuthoritativeIdle,
    attachToSession: (sessionId: string) => attachToSession(client, sessionId, options.clientId),
    setAttachmentState,
    kernelEventSubscriptionController,
    syncKernelEventSubscription,
    transitionToNoSession,
    queueTerminalOutputRecords,
    scheduleSharedPromptInputHistoryRefresh,
    handleWaitingRoomRefresh: refreshWaitingRoomData,
    applyWaitingRoomRowsChanged,
    applyRelayStatusChanged,
    applyRemoteMachinesChanged,
    applyProviderCatalogChanged,
    applySlicesChanged,
    recoverAttachedSessionAfterKernelRestart,
    setFatalError,
    pumpTerminalOutput: (sessionId: string, attachmentId: string) => pumpTerminalOutput(client, sessionId, attachmentId),
    pollRuntimeNotices: (sessionId: string, attachmentId: string) => pollRuntimeNotices(client, sessionId, attachmentId),
    promptInputRefController,
    transcriptScrollboxRefController,
    primaryTranscriptRuntimeStore,
    syncPromptPlaceholder,
    logViewDebug,
    footerFlashController,
    clearPendingPromptDraftPersist,
    cancelPendingTurnCompletion,
    sessionChromeUpdateController,
    promptInputHistoryRefreshController,
    transcriptHistoryAutoloadController,
    setWorkingAnimationFrame,
    sessionStatusMode,
    workspaceLiveSyncStatus,
    renderSplitPaneFooters,
    hydrateCurrentAttachedSession,
  })
  recordDaemonActivity = runtimeRecordDaemonActivity
  setKernelTerminalOutputRecordProcessor(runtimeProcessKernelTerminalOutputRecord)

  return (
    <CliAppWorkspaceView
      width={dimensions().width}
      height={dimensions().height}
      fatalError={fatalError() !== null}
      themeRevision={themeRevision()}
      responsePaneRows={responsePaneRows}
      promptPlaceholder={promptPlaceholder()}
      promptInputMaxHeight={promptInputMaxHeight()}
      promptAreaBackground={promptAreaBackground()}
      retainPromptFocus={retainPromptFocus}
      handlePromptSelectionSurfaceMouseUp={handlePromptSelectionSurfaceMouseUp}
      responsePaneRenderRefStore={responsePaneRenderRefStore}
      historyLoadingRenderController={historyLoadingRenderController}
      transcriptScrollboxRefController={transcriptScrollboxRefController}
      commandCenterController={commandCenterController}
      promptInputRefController={promptInputRefController}
      promptTextController={promptTextController}
      assignPromptMetaRef={assignPromptMetaRef}
      assignStatusIndicatorBox={assignStatusIndicatorBox}
      assignFooterSummaryBox={assignFooterSummaryBox}
      assignDialogOverlayBox={assignDialogOverlayBox}
      handlePromptKeyDown={handlePromptKeyDown}
      handlePromptContentChange={handlePromptContentChange}
      focusedAgentInteraction={focusedAgentInteraction}
      submitFocusedInteractionChoice={submitFocusedInteractionChoice}
      commandCenterOpen={commandCenterOpen}
      selectCommandCenterFromSubmit={selectCommandCenterFromSubmit}
      submitPrompt={submitPrompt}
      logViewDebug={logViewDebug}
      applyResponseLayout={applyResponseLayout}
      renderHistoryLoadingIndicator={renderHistoryLoadingIndicator}
      rebuildTranscript={rebuildTranscript}
      ensureBackgroundPollersStarted={ensureBackgroundPollersStarted}
      renderAgentInteractions={renderAgentInteractions}
      renderSplitPaneFooters={renderSplitPaneFooters}
      renderCommandCenter={renderCommandCenter}
      syncPromptPlaceholder={syncPromptPlaceholder}
      setPromptText={setPromptText}
      syncPromptTextSnapshot={syncPromptTextSnapshot}
      refreshPromptAttachmentHighlights={refreshPromptAttachmentHighlights}
      updateSessionChrome={updateSessionChrome}
      renderHotkeysOverlay={renderHotkeysOverlay}
    />
  )
}

function shouldRefreshWorkflowRegistrySuggestions(input: string): boolean {
  const normalized = input.trimStart()
  return normalized.startsWith("/workflow load ")
    || normalized.startsWith("/workflow run ")
    || normalized.startsWith("/workflow registry get ")
    || normalized.startsWith("/workflow registry delete ")
}

function shouldInvalidateWorkflowRegistrySuggestions(command: string): boolean {
  const normalized = command.trimStart()
  return normalized.startsWith("/workflow load ")
    || normalized.startsWith("/workflow run ")
    || normalized.startsWith("/workflow registry add ")
    || normalized.startsWith("/workflow registry add-from-workflow ")
    || normalized.startsWith("/workflow registry delete ")
}
