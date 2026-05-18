import process from "node:process"
import { randomBytes } from "node:crypto"
import { homedir } from "node:os"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, ScrollBoxRenderable, TextRenderable, type TextareaRenderable } from "@opentui/core"
import { useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect, onCleanup } from "solid-js"
import { reconcile } from "solid-js/store"

import type {
  BootstrapState,
  TerminalOutputRecord,
} from "./cli-types.js"
import { createCliAgentPaneComposition } from "./cli-agent-pane-composition.js"
import { createCliBackgroundRuntimeComposition } from "./cli-background-runtime-composition.js"
import { createCliAutomationProcessComposition } from "./cli-automation-process-composition.js"
import { createCliAppState } from "./cli-app-state.js"
import { createCliCommandActionComposition } from "./cli-command-action-composition.js"
import { createCliInputRoutingComposition } from "./cli-input-routing-composition.js"
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
  PROMPT_KEYBINDINGS,
} from "./cli-runtime-tuning.js"
import { createAgentFocusTransitionController } from "./agent-focus-transition-controller.js"
import {
  createCliRendererFocusController,
} from "./cli-renderer-focus-controller.js"
import { createCommandCenterCommandExecutor } from "./command-center-command-executor.js"
import { createCommandCenterLayoutController } from "./command-center-layout-controller.js"
import { createCommandCenterController } from "./command-center-controller.js"
import { renderCommandCenterOverlay } from "./command-center-renderer.js"
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
  promptAttachmentTokenStyle,
} from "./prompt-attachment-tokens.js"
import {
  cancelActivePrompt,
} from "./prompt-runtime-api.js"
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
  createWorkflowController,
  createWorkflowSelectionSyncController,
} from "./workflow-controller.js"
import {
  createWorkflowInspectorController,
} from "./workflow-inspector-controller.js"
import {
  createWorkflowNodeInstructionsEditorController,
} from "./workflow-node-instructions-editor-controller.js"
import { createWorkflowTerminalPanelController } from "./workflow-terminal-panel-controller.js"
import { WorkspaceLayout } from "./workspace-layout.js"
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
    selectedWorkflowId,
    setSelectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
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
      ? initialSession.focused_agent_id ?? initialSession.agents[0]?.id ?? null
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
    workflowScreenActive: () => workflowScreenActive(),
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
  const workflowSelectionSyncController = createWorkflowSelectionSyncController({
    workflows: () => sessionState().workflows ?? [],
    selectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowId,
    setSelectedWorkflowNodeId,
  })
  createEffect(() => {
    workflowSelectionSyncController.sync()
  })
  const workflowInspectorController = createWorkflowInspectorController({
    getSession: sessionState,
    getSelectedWorkflowId: selectedWorkflowId,
    getSelectedWorkflowNodeId: selectedWorkflowNodeId,
    getInspectorMode: workflowInspectorMode,
    getNodeInstructionsEditor: workflowNodeInstructionsEditor,
    updateNodeInstructionsDraft: (draft) => workflowNodeInstructionsEditorController.updateDraft(draft),
    setNodeInstructionsInputRef: (editorRef) => {
      workflowNodeInstructionsEditorController.setInputRef(editorRef)
    },
  })
  const workflowInspector = workflowInspectorController.project
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
      streamingAgentId: streamingAgentId(),
      activePromptAgentId: activePrompt()?.target_agent_id ?? null,
      agents: sessionState().agents,
      focusedAgentId: focusedAgentId(),
    })
  }
  const rendererFocusController = createCliRendererFocusController(renderer)
  const describeRenderableDebug = rendererFocusController.describe
  const currentFocusedRenderable = rendererFocusController.current
  const {
    activateWaitingRoom,
    applyModelSelection,
    applyProviderSelection,
    applyVariantSelection,
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
  const commandCenterController = createCommandCenterController<BoxRenderable>({
    getProviderCatalog: providerCatalogState,
    getProviderCommandCatalogs: providerCommandCatalogState,
    getCurrentProvider: () => normalizeBackendProviderId(currentProviderSelection().provider),
    getFocusedProvider: focusedBackendProvider,
    getCurrentModel: currentModelId,
    getCurrentVariant: currentVariantId,
    getPromptText: promptTextController.currentText,
    replacePromptText: promptTextController.setText,
    executeCommand: (command) => executeCommandCenterCommand(command),
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
    workflowScreenActive: () => workflowScreenActive(),
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
    agentLocationLabel,
    workingAnimationFrame,
    activeInteractionForAgent,
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
    agentPaneRuntimeStore,
    transcriptSyntaxStyleController,
    auxiliaryTranscriptSurfaceTone,
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
    updateSessionChrome,
    flashFooter,
    attachmentState,
    sessionState,
    selectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
    workflowScreenActive: () => workflowScreenActive(),
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

  const workflowNodeInstructionsEditorController = createWorkflowNodeInstructionsEditorController({
    getEditor: workflowNodeInstructionsEditor,
    setEditor: setWorkflowNodeInstructionsEditor,
    workflowScreenShowing,
    setWorkspaceScreenMode,
    rebuildTranscript,
    scheduleTimer: startTimeout,
    focusPromptInput: () => {
      promptInputRefController.focus()
    },
  })
  const openWorkflowNodeInstructionsEditor = workflowNodeInstructionsEditorController.open
  const closeWorkflowNodeInstructionsEditor = workflowNodeInstructionsEditorController.close
  const getWorkflowNodeInstructionsContext = workflowNodeInstructionsEditorController.context
  const getWorkflowNodeInstructionsDraft = workflowNodeInstructionsEditorController.draft

  const openWorkflowTerminalPanel = createWorkflowTerminalPanelController({
    clearNodeInstructionsEditor: workflowNodeInstructionsEditorController.clear,
    setWorkflowInspectorMode,
    setSelectedWorkflowId,
    workflowScreenShowing,
    setWorkspaceScreenMode,
    rebuildTranscript,
  }).open

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
    setAvailableSessions,
    scheduleShortViewportHistoryCheck: () => transcriptHistoryAutoloadController.scheduleShortViewportCheck(),
    updateSessionChrome,
    appendNotice,
    flashFooter,
    logProviderRunDebug,
  })

  const {
    workflowScreenActive,
    toggleWorkspaceScreen,
    showWorkflowScreen,
    selectWorkflowCanvas,
    cycleWorkflowCanvasNode,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    invokeWorkflowEndpoint,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    setWorkflowIntermediateOutputSchema,
    listWorkflowRuns,
    cancelWorkflowRun,
    resumeWorkflowRun,
  } = createWorkflowController({
    sendRequest: (request) => client.send<Record<string, unknown>>(request),
    isAttached,
    sessionState,
    applySessionState,
    selectedWorkflowId,
    setSelectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
    workspaceScreenMode,
    setWorkspaceScreenMode,
    rebuildTranscript,
    applyResponseLayout,
  })

  const {
    handleSessionCommand,
    handleProviderCommand,
    handleModelCommand,
    handleVariantCommand,
    handleViewCommand,
    handleCycleAgentFocus,
    handleAgentCommand,
    handleKernelCommand,
    handleMachineCommand,
    handleSliceCommand,
    handleRelayCommand,
    handleCloudCommand,
    handleConfigCommand,
    handleWorkspaceCommand,
    handleWorktreeCommand,
    handleWorkflowCommand,
    handleMcpCommand,
    handleSkillCommand,
  } = createCliCommandActionComposition({
    client,
    options,
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
    formatError,
    attachBinding,
    transitionToNoSession,
    applyProviderSelection,
    applyModelSelection,
    applyVariantSelection,
    refreshWaitingRoomData,
    setSlicesState,
    appLogger,
    setMultiAgentResponseLayout,
    applyResponseLayout,
    applySessionState,
    refreshAgentPanes,
    openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor,
    getWorkflowNodeInstructionsDraft,
    getWorkflowNodeInstructionsContext,
    openWorkflowTerminalPanel,
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
    workflowScreenActive,
    showWorkflowScreen,
    selectedWorkflowId,
    selectWorkflowCanvas,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    invokeWorkflowEndpoint,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    setWorkflowIntermediateOutputSchema,
    listWorkflowRuns,
    cancelWorkflowRun,
    resumeWorkflowRun,
    refreshSplitPaneFocusRepaint,
  })

  const commandCenterCommandExecutor = createCommandCenterCommandExecutor({
    onExit: () => requestExit(),
    onWaiting: () => requestWaitingRoom(),
    onStop: () => requestPromptStop(),
    handleAttachmentCommand,
    onSession: handleSessionCommand,
    onProvider: handleProviderCommand,
    onModel: handleModelCommand,
    onVariant: handleVariantCommand,
    onView: handleViewCommand,
    onAgent: handleAgentCommand,
    onKernel: handleKernelCommand,
    onMachine: handleMachineCommand,
    onSlice: handleSliceCommand,
    onRelay: handleRelayCommand,
    onCloud: handleCloudCommand,
    onConfig: handleConfigCommand,
    onWorkspace: handleWorkspaceCommand,
    onWorktree: handleWorktreeCommand,
    onWorkflow: handleWorkflowCommand,
    onMcp: handleMcpCommand,
    onSkill: handleSkillCommand,
    flashFooter,
    formatError,
  })
  const executeCommandCenterCommand = commandCenterCommandExecutor.execute

  const {
    cycleFocusedInteractionChoice,
    handlePromptKeyDown,
    handleSigint,
    handleStdinData,
    requestPromptStop,
    submitFocusedInteractionChoice,
    submitPrompt,
    submitWorkspaceShellCommand,
  } = createCliInputRoutingComposition({
    client,
    options,
    appLogger,
    formatError,
    isAttached,
    sessionState,
    recordPromptAreaHistoryEntry,
    promptTextController,
    setPromptHistoryIndex,
    setPromptHistoryDraft,
    clearCommandCenter,
    flashFooter,
    requestExit,
    requestWaitingRoom,
    promptStopController,
    handleAttachmentCommand,
    handleSessionCommand,
    handleProviderCommand,
    handleModelCommand,
    handleVariantCommand,
    handleViewCommand,
    handleAgentCommand,
    handleKernelCommand,
    handleMachineCommand,
    handleSliceCommand,
    handleRelayCommand,
    handleCloudCommand,
    handleConfigCommand,
    handleWorkspaceCommand,
    handleWorktreeCommand,
    handleWorkflowCommand,
    handleMcpCommand,
    handleSkillCommand,
    workspaceShellContext,
    setWorkspaceShellContext,
    workspaceShellEntryCounter,
    setWorkspaceShellEntryCounter,
    setWorkspaceShellEntries,
    applySessionState,
    selectedWorkflowId,
    setSelectedWorkflowId,
    setSelectedWorkflowNodeId,
    rebuildTranscript,
    workflowPromptState,
    pendingAttachments,
    beginSubmittedPromptUi,
    restoreFailedPromptUi,
    invokeWorkflowEndpoint,
    focusedBackendProvider,
    workflowScreenShowing,
    waitForPendingAgentFocusTransition,
    focusedAgentId,
    primaryTranscriptRuntimeStore,
    setProviderActivityLabel,
    setActiveStatusLabel,
    attachmentState,
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
    focusedAgentInteraction,
    interactionChoiceStore,
    renderAgentInteractions,
    applyResponseLayout,
    handleHotkeysToggleShortcut,
    dialogOverlayOpen,
    closeActiveDialogOverlay,
    activePrompt,
    handleCommandCenterKey,
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
    remoteKernelsState,
    terminalsState,
    slicesState,
    themeRegistryState,
    reconcileWaitingRoom,
    setWaitingRoomState,
    applyWaitingRoomSessionLifecycleAction,
    activateWaitingRoom,
    handleSessionBrowserKey,
    toggleWorkspaceScreen,
    workflowScreenActive,
    cycleWorkflowCanvasNode,
    handleCycleAgentFocus,
    copyPromptSelection,
    removePromptAttachmentsForEdit,
    removeLastPendingPromptAttachment,
  })

  const automationProcessComposition = createCliAutomationProcessComposition({
    client,
    options,
    appLogger: appLogger ?? null,
    formatError,
    flashFooter,
    handleSigint,
    handleStdinData,
    onSigint: (handler) => process.on("SIGINT", handler),
    offSigint: (handler) => process.off("SIGINT", handler),
    onStdinData: (handler) => process.stdin.on("data", handler),
    offStdinData: (handler) => process.stdin.off("data", handler),
    clearTerminalOutputRecordTimer,
    workspaceScreenMode,
    workflowScreenActive,
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
    availableSessions,
    providerCatalogState,
    waitingRoomCloudNotice,
    waitingRoomInventoryStatus,
    relayStatusState,
    remoteMachinesState,
    remoteKernelsState,
    terminalsState,
    slicesState,
    waitingRoomTargets,
    themeRegistryState,
    selectedWorkflowId,
    selectedWorkflowNodeId,
    workspaceShellContext,
    workspaceShellEntries,
    footerFlash,
    getInteractionChoiceSelection: interactionChoiceStore.getSelectedIndex,
    getInteractionCustomReply: interactionChoiceStore.getStoredCustomReply,
    isInteractionCustomEditing: interactionChoiceStore.isCustomEditing,
    kernelConnected,
    setWorkspaceScreenMode,
    rebuildTranscript,
    applyResponseLayout,
    showWorkflowScreen,
    submitWorkspaceShellCommand,
    attachmentState,
    setPromptText,
    submitPrompt,
    activateWaitingRoom,
    connectDetachedKernelFromWaitingRoom,
    submitFocusedInteractionChoice,
    cycleFocusedInteractionChoice,
    restoreTerminalAndExit,
    sleep,
  })
  automationProcessComposition.start()
  onCleanup(automationProcessComposition.stop)

  const {
    recordDaemonActivity: runtimeRecordDaemonActivity,
    ensureBackgroundPollersStarted,
    processKernelTerminalOutputRecord: runtimeProcessKernelTerminalOutputRecord,
  } = createCliBackgroundRuntimeComposition({
    client,
    appLogger,
    formatError,
    sleep,
    scheduleInterval: startInterval,
    clearInterval,
    closingStateController,
    isAttached,
    sessionState,
    resizeSession: (sessionId) => maybeResize(client, sessionId),
    setDaemonDisconnected,
    setStatusLine,
    updateSessionChrome,
    appendNotice,
    working,
    supportsKernelEventStream,
    recoverProviderRun,
    daemonDisconnected,
    recordTurnActivity,
    resolveTerminalRecordAgentId,
    setStreamingAgentId,
    markAgentBusy,
    splitAgentResponseMode,
    visibleTranscriptAgentId,
    focusedAgentId,
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
    attachmentState,
    catchUpAttachedSession: (sessionId, attachmentId, session) =>
      catchUpAttachedSession(client, sessionId, attachmentId, session, appLogger),
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    tryGetProviderRun: (providerRunId) => tryGetProviderRun(client, providerRunId, appLogger),
    clearLocalBusyStateForAuthoritativeIdle,
    attachToSession: (sessionId) => attachToSession(client, sessionId, options.clientId),
    setAttachmentState,
    kernelEventSubscriptionController,
    syncKernelEventSubscription,
    transitionToNoSession,
    queueTerminalOutputRecords,
    scheduleSharedPromptInputHistoryRefresh,
    handleWaitingRoomRefresh: refreshWaitingRoomData,
    flashFooter,
    recoverAttachedSessionAfterKernelRestart,
    setFatalError,
    pumpTerminalOutput: (sessionId, attachmentId) => pumpTerminalOutput(client, sessionId, attachmentId),
    pollRuntimeNotices: (sessionId, attachmentId) => pollRuntimeNotices(client, sessionId, attachmentId),
    promptInputRefController,
    transcriptScrollboxRefController,
    primaryTranscriptRuntimeStore,
    rebuildTranscript,
    syncPromptPlaceholder,
    addResizeListener: (handler) => {
      process.stdout.on("resize", handler)
    },
    removeResizeListener: (handler) => {
      process.stdout.off("resize", handler)
    },
    logViewDebug,
    footerFlashController,
    clearPendingPromptDraftPersist,
    cancelPendingTurnCompletion,
    sessionChromeUpdateController,
    promptInputHistoryRefreshController,
    transcriptHistoryAutoloadController,
    setWorkingAnimationFrame,
    sessionStatusMode,
    renderSplitPaneFooters,
    waitingRoomState,
    setWaitingRoomState,
    kernelConnected,
    hydrateCurrentAttachedSession,
  })
  recordDaemonActivity = runtimeRecordDaemonActivity
  setKernelTerminalOutputRecordProcessor(runtimeProcessKernelTerminalOutputRecord)

  return (
    <WorkspaceLayout
      width={dimensions().width}
      height={dimensions().height}
      fatalError={fatalError() !== null}
      themeRevision={themeRevision()}
      responsePaneRows={responsePaneRows}
      promptPlaceholder={promptPlaceholder()}
      promptInputMaxHeight={promptInputMaxHeight()}
      promptAreaBackground={promptAreaBackground()}
      promptKeyBindings={PROMPT_KEYBINDINGS}
      onRootMouseUp={retainPromptFocus}
      onResponseSurfaceMouseUp={handlePromptSelectionSurfaceMouseUp}
      onFooterMouseUp={handlePromptSelectionSurfaceMouseUp}
      onResponseLayoutBoxRef={(value) => {
        responsePaneRenderRefStore.assignLayoutBox(value)
        logViewDebug("mounted response layout box")
        applyResponseLayout()
      }}
      onResponseRowBoxRef={(index, value) => {
        responsePaneRenderRefStore.assignRowBox(index, value)
        applyResponseLayout()
      }}
      onPaneGridBorderRowRef={(index, value) => {
        responsePaneRenderRefStore.assignBorderRow(index, value)
        applyResponseLayout()
      }}
      onPaneGridBottomBorderRowRef={(value) => {
        responsePaneRenderRefStore.assignBottomBorderRow(value)
        applyResponseLayout()
      }}
      onPaneGridHorizontalSegmentRef={(rowIndex, segmentIndex, value) => {
        responsePaneRenderRefStore.assignHorizontalSegment(rowIndex, segmentIndex, value)
        applyResponseLayout()
      }}
      onPaneGridBottomHorizontalSegmentRef={(segmentIndex, value) => {
        responsePaneRenderRefStore.assignBottomHorizontalSegment(segmentIndex, value)
        applyResponseLayout()
      }}
      onPaneGridJunctionTextRef={(rowIndex, junctionIndex, value) => {
        responsePaneRenderRefStore.assignJunctionText(rowIndex, junctionIndex, value)
        applyResponseLayout()
      }}
      onPaneGridBottomJunctionTextRef={(junctionIndex, value) => {
        responsePaneRenderRefStore.assignBottomJunctionText(junctionIndex, value)
        applyResponseLayout()
      }}
      onPaneGridVerticalSegmentRef={(rowIndex, segmentIndex, value) => {
        responsePaneRenderRefStore.assignVerticalSegment(rowIndex, segmentIndex, value)
        applyResponseLayout()
      }}
      onResponsePrimaryPaneRef={(value) => {
        responsePaneRenderRefStore.assignPrimaryPane(value)
        logViewDebug("mounted response primary pane")
        applyResponseLayout()
      }}
      onHistoryLoadingBoxRef={(value) => {
        historyLoadingRenderController.assignBox(value)
        logViewDebug("mounted history loading box")
        renderHistoryLoadingIndicator()
      }}
      onTranscriptScrollboxRef={(value) => {
        transcriptScrollboxRefController.assignScrollbox(value)
        logViewDebug("mounted primary transcript scrollbox")
        rebuildTranscript()
        ensureBackgroundPollersStarted()
      }}
      onResponsePrimaryInteractionBoxRef={(value) => {
        responsePaneRenderRefStore.assignPrimaryInteractionBox(value)
        renderAgentInteractions()
        applyResponseLayout()
      }}
      onResponsePrimaryFooterBoxRef={(value) => {
        responsePaneRenderRefStore.assignPrimaryFooterBox(value)
        renderSplitPaneFooters()
        applyResponseLayout()
      }}
      onResponseAuxiliaryPaneRef={(index, value) => {
        responsePaneRenderRefStore.assignAuxiliaryPane(index, value)
        logViewDebug("mounted response auxiliary pane", {
          pane_index: index + 1,
        })
        applyResponseLayout()
      }}
      onResponseAuxiliaryScrollboxRef={(index, value) => {
        responsePaneRenderRefStore.assignAuxiliaryScrollbox(index, value)
        logViewDebug("mounted response auxiliary scrollbox", {
          pane_index: index + 1,
        })
        applyResponseLayout()
      }}
      onResponseAuxiliaryInteractionBoxRef={(index, value) => {
        responsePaneRenderRefStore.assignAuxiliaryInteractionBox(index, value)
        renderAgentInteractions()
        applyResponseLayout()
      }}
      onResponseAuxiliaryFooterBoxRef={(index, value) => {
        responsePaneRenderRefStore.assignAuxiliaryFooterBox(index, value)
        renderSplitPaneFooters()
        applyResponseLayout()
      }}
      onCommandCenterBoxRef={(value) => {
        commandCenterController.assignBox(value)
        renderCommandCenter()
      }}
      onPromptInputRef={(value) => {
        promptInputRefController.assignInput(value)
        promptInputRefController.setSyntaxStyle(promptAttachmentTokenStyle)
        syncPromptPlaceholder()
        if (promptTextController.snapshot()) {
          setPromptText(promptTextController.snapshot())
        }
        syncPromptTextSnapshot()
        refreshPromptAttachmentHighlights()
        ensureBackgroundPollersStarted()
      }}
      onPromptKeyDown={handlePromptKeyDown}
      onPromptContentChange={handlePromptContentChange}
      onPromptSubmit={() => {
        if (focusedAgentInteraction()) {
          void submitFocusedInteractionChoice()
          return
        }
        if (commandCenterOpen()) {
          if (selectCommandCenterFromSubmit()) {
            return
          }
        }
        void submitPrompt()
      }}
      onPromptMetaProviderTextRef={assignPromptMetaRef("providerText")}
      onPromptMetaProviderDividerTextRef={assignPromptMetaRef("providerDividerText")}
      onPromptMetaModelTextRef={assignPromptMetaRef("modelText")}
      onPromptMetaModelDividerTextRef={assignPromptMetaRef("modelDividerText")}
      onPromptMetaVariantTextRef={assignPromptMetaRef("variantText")}
      onPromptMetaUsageDividerTextRef={assignPromptMetaRef("usageDividerText")}
      onPromptMetaUsageTokensTextRef={assignPromptMetaRef("usageTokensText")}
      onPromptMetaUsageBarOpenTextRef={assignPromptMetaRef("usageBarOpenText")}
      onPromptMetaUsageBarFilledTextRef={assignPromptMetaRef("usageBarFilledText")}
      onPromptMetaUsageBarEmptyTextRef={assignPromptMetaRef("usageBarEmptyText")}
      onPromptMetaUsageBarCloseTextRef={assignPromptMetaRef("usageBarCloseText")}
      onPromptMetaUsagePercentTextRef={assignPromptMetaRef("usagePercentText")}
      onStatusIndicatorBoxRef={(value) => {
        assignStatusIndicatorBox(value)
        updateSessionChrome()
      }}
      onFooterSummaryBoxRef={(value) => {
        assignFooterSummaryBox(value)
        updateSessionChrome()
      }}
      onHotkeysOverlayBoxRef={(value) => {
        assignDialogOverlayBox(value)
        renderHotkeysOverlay()
      }}
    />
  )
}
