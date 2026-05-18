import process from "node:process"
import { randomBytes } from "node:crypto"
import { homedir } from "node:os"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, ScrollBoxRenderable, TextAttributes, TextRenderable, addDefaultParsers, type TextareaRenderable } from "@opentui/core"
import { render, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect, createMemo, onCleanup, onMount } from "solid-js"
import { reconcile } from "solid-js/store"

import type {
  BootstrapState,
  RuntimeSession,
  TerminalOutputRecord,
} from "./cli-types.js"
import { createCliBackgroundRuntimeComposition } from "./cli-background-runtime-composition.js"
import { createCliAutomationProcessComposition } from "./cli-automation-process-composition.js"
import { createCliAppState } from "./cli-app-state.js"
import { createCliCommandActionComposition } from "./cli-command-action-composition.js"
import { createCliInputRoutingComposition } from "./cli-input-routing-composition.js"
import { createCliOverlayInteractionComposition } from "./cli-overlay-interaction-composition.js"
import { createCliPromptSurfaceComposition } from "./cli-prompt-surface-composition.js"
import { createCliWaitingRoomComposition } from "./cli-waiting-room-composition.js"
import { createAgentInteractionStripController } from "./agent-interaction-strip-controller.js"
import { createAttachedSessionPrimeController } from "./attached-session-prime-controller.js"
import { createAssistantMessageCompletionController } from "./assistant-message-completion-controller.js"
import { createAuthoritativeIdleController } from "./authoritative-idle-controller.js"
import { createCliClosingStateController } from "./cli-closing-state-controller.js"
import {
  CHROME_UPDATE_THROTTLE_MS,
  COMMAND_CENTER_OVERLAY_FOOTPRINT,
  LIVE_TRANSCRIPT_LIMIT,
  LIVE_TRANSCRIPT_MAX_CHARS,
  PROMPT_KEYBINDINGS,
  STREAM_BATCH_WINDOW_MS,
  TURN_COMPLETION_QUIET_MS,
} from "./cli-runtime-tuning.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import { createAgentFocusTransitionController } from "./agent-focus-transition-controller.js"
import { createAgentRuntimeProjectionController } from "./agent-runtime-projection-controller.js"
import {
  createCliRendererFocusController,
} from "./cli-renderer-focus-controller.js"
import { createCliLoadingStateController } from "./cli-loading-state-controller.js"
import { createCommandCenterCommandExecutor } from "./command-center-command-executor.js"
import { createCommandCenterLayoutController } from "./command-center-layout-controller.js"
import { createCommandCenterController } from "./command-center-controller.js"
import { renderCommandCenterOverlay } from "./command-center-renderer.js"
import { createAgentPaneRefreshController } from "./agent-pane-refresh-controller.js"
import { createAgentPaneRuntimeResetController } from "./agent-pane-runtime-reset-controller.js"
import { createAgentPaneRuntimeStoreController } from "./agent-pane-runtime-store-controller.js"
import { createAgentPaneStoreController } from "./agent-pane-store-controller.js"
import { createAgentPaneTranscriptEntryController } from "./agent-pane-transcript-entry-controller.js"
import { createAgentPaneTranscriptInteractionController } from "./agent-pane-transcript-interaction-controller.js"
import { createAgentPaneTranscriptRenderController } from "./agent-pane-transcript-render-controller.js"
import { createAgentPaneTranscriptRetentionController } from "./agent-pane-transcript-retention-controller.js"
import { createAgentPaneTranscriptStreamController } from "./agent-pane-transcript-stream-controller.js"
import { createAgentPaneStreamingCommitController } from "./agent-pane-streaming-commit-controller.js"
import { createFooterFlashController } from "./footer-flash-controller.js"
import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"
import { createHistoryLoadingRenderController } from "./history-loading-render-controller.js"
import { createHistoryScrollRestoreController } from "./history-scroll-restore-controller.js"
import { clampScrollTop } from "./history-viewport.js"
import { renderHistoryLoadingIndicator as renderHistoryLoadingIndicatorView } from "./history-loading-renderer.js"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import {
  createInteractionProjectionController,
} from "./interaction-projection-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"
import { runClaudeNativeTui } from "./native-tui/claude.js"
import { runCodexNativeTui } from "./native-tui/codex.js"
import { runOpenCodeNativeTui } from "./native-tui/opencode.js"
import { createKernelEventSubscriptionController } from "./kernel-event-subscription-controller.js"
import { createKernelRestartRecoveryController } from "./kernel-restart-recovery-controller.js"
import {
  createCliProcessLoggerRegistry,
  formatCliError,
} from "./cli-process-logging.js"
import { runLogViewer } from "./logs.js"
import {
  bootstrapCliRuntime,
} from "./cli-runtime-bootstrap.js"
import {
  createCliRuntimeDebugLogger,
} from "./cli-runtime-debug-logger.js"
import {
  createCliUiBatchController,
} from "./cli-ui-batch-controller.js"
import { createCliExitController } from "./cli-exit-controller.js"
import { createPromptAttachmentIntakeController } from "./prompt-attachment-intake-controller.js"
import { createPromptSubmissionAgentStateController } from "./prompt-submission-agent-state-controller.js"
import {
  createPromptTextController,
} from "./prompt-text-controller.js"
import { createPromptStopController } from "./prompt-stop-controller.js"
import { createPrimaryTranscriptEntryController } from "./primary-transcript-entry-controller.js"
import { createPrimaryTranscriptRenderController } from "./primary-transcript-render-controller.js"
import { createPrimaryTranscriptRuntimeStoreController } from "./primary-transcript-runtime-store-controller.js"
import {
  createTurnCompletionController,
} from "./turn-completion-controller.js"
import {
  promptAttachmentTokenStyle,
} from "./prompt-attachment-tokens.js"
import {
  cancelActivePrompt,
} from "./prompt-runtime-api.js"
import {
  getSessionHistory,
} from "./session-history-api.js"
import { createPromptMetaRenderController } from "./prompt-meta-render-controller.js"
import { renderPromptMeta } from "./prompt-meta-renderer.js"
import {
  type BackendProviderId,
  normalizeBackendProviderId,
} from "./provider-catalog.js"
import { createProviderActivityController } from "./provider-activity-controller.js"
import {
  getProviderCatalog,
  getProviderRun,
  launchProviderRun,
  tryGetProviderRun,
} from "./provider-api.js"
import { createProviderRecoveryController } from "./provider-recovery-controller.js"
import { createPromptInputRefController } from "./prompt-input-ref-controller.js"
import { createResponseLayoutController } from "./response-layout-controller.js"
import { createResponsePaneProjectionController } from "./response-pane-projection-controller.js"
import { createResponsePaneRenderRefStoreController } from "./response-pane-render-ref-store-controller.js"
import { createResponsePaneRenderScheduleController } from "./response-pane-render-schedule-controller.js"
import {
  splitPaneAuxiliaryAgentIds,
} from "./response-panes.js"
import {
  extractPromptHistoryEntries,
} from "./prompt-history.js"
import {
  STATUS_BADGE_WIDTH,
  DEFAULT_CONNECTED_STATUS,
  getExitCleanupDecision,
  getSessionStatusLabel,
  getTurnCompletionDelayMs,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
import {
  applyProviderRunProfileToSession,
} from "./session-chrome-state.js"
import { createFocusedStatusBadgeController } from "./focused-status-badge-controller.js"
import {
  createSessionChromeRenderController,
} from "./session-chrome-render-controller.js"
import {
  createSessionChromeSummaryRenderState,
  renderSessionChromeSummary,
} from "./session-chrome-summary-renderer.js"
import {
  createSessionChromeUpdateController,
  type SessionChromeUpdateController,
} from "./session-chrome-update-controller.js"
import {
  agentHasPromptWork,
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  focusedAgentIdForSession,
  sessionHasPromptWork,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "./session-state.js"
import { createSessionStateApplyController } from "./session-state-apply-controller.js"
import { createSessionAttachmentController } from "./session-attachment-controller.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createTranscriptHistoryLoadController } from "./transcript-history-load-controller.js"
import { resolveTerminalRecordAgentId as resolveTerminalRecordAgentIdFromState } from "./terminal-record-agent-resolver.js"
import { createTranscriptHistoryAutoloadController } from "./transcript-history-autoload-controller.js"
import { createTranscriptScrollboxRefController } from "./transcript-scrollbox-ref-controller.js"
import { createTranscriptEntryProjectionController } from "./transcript-entry-projection-controller.js"
import {
  createTerminalOutputRecordQueue,
} from "./terminal-output-record-queue.js"
import { createTerminalOutputRecordProcessor } from "./terminal-output-record-processor.js"
import { createTerminalExitController } from "./terminal-exit-controller.js"
import { createTranscriptViewportController } from "./transcript-viewport-controller.js"
import { createTranscriptRenderDeferralController } from "./transcript-render-deferral-controller.js"
import { createTranscriptParserRegistration } from "./transcript-parser-registration.js"
import { createVisibleActivityLabelController } from "./visible-activity-label-controller.js"
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
  renderSplitPaneFooters as renderSplitPaneFootersView,
} from "./split-pane-footer-renderer.js"
import { createSplitPaneFooterRenderController } from "./split-pane-footer-render-controller.js"
import {
  createStatusIndicatorRenderState,
  renderStatusIndicator as renderStatusIndicatorView,
} from "./status-indicator-renderer.js"
import { createStatusIndicatorController } from "./status-indicator-controller.js"
import { createRenderScheduler } from "./render-scheduler.js"
import {
  createResponsePaneRepaintController,
} from "./response-pane-repaint-controller.js"
import { createTranscriptSyntaxStyle, theme } from "./theme.js"
import { createWaitingRoomTransitionController } from "./waiting-room-transition-controller.js"
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
  deriveWorkflowPromptState,
} from "./workflow-prompt-state.js"
import {
  createWorkflowNodeInstructionsEditorController,
} from "./workflow-node-instructions-editor-controller.js"
import { createWorkflowTerminalPanelController } from "./workflow-terminal-panel-controller.js"
import { WorkspaceLayout } from "./workspace-layout.js"
import {
  computeCurrentTurnId,
  computeNextTurnId,
  formatTranscriptPreview,
} from "./transcript-preview.js"
import {
  buildTranscriptEntryRenderable,
  resolveTranscriptSurfaceTone,
  transcriptRenderMode,
  transcriptSurfacePalette,
  type TranscriptEntryRenderable,
  type TranscriptSurfaceTone,
} from "./transcript-render.js"
import { createTranscriptRetentionController } from "./transcript-retention-controller.js"
import { createTranscriptEventController } from "./transcript-event-controller.js"
import { createTranscriptStateController } from "./transcript-state-controller.js"
import { createTranscriptStreamController } from "./transcript-stream-controller.js"
import { createTranscriptSyntaxStyleController } from "./transcript-syntax-style-controller.js"
import { createTranscriptTurnStateController } from "./transcript-turn-state-controller.js"
import { createTranscriptTurnExpansionController } from "./transcript-turn-expansion-controller.js"
import {
  buildEmptyTranscriptRenderable,
  buildLoadingTranscriptRenderable,
  buildNoSessionRenderable,
  buildWorkflowOutlineRenderable,
} from "./workspace-renderables.js"
import parserConfig from "./parsers-config.js"

const DEBUG_LOGS_ENABLED = (process.env.ARROBA_LOG_LEVEL ?? "").toLowerCase() === "debug"
const OPEN_CONSOLE_ON_ERROR = process.env.ARROBA_OPEN_CONSOLE_ON_ERROR === "1"
const processLoggers = createCliProcessLoggerRegistry()
const getLogger = processLoggers.getLogger
const formatError = formatCliError
const transcriptParserRegistration = createTranscriptParserRegistration({
  parsers: parserConfig.parsers,
  addDefaultParsers: (parsers) => {
    addDefaultParsers([...parsers])
  },
})

async function main() {
  const argv = process.argv.slice(2)
  if (argv[0] === "logs") {
    await runLogViewer(argv.slice(1))
    return
  }
  if (argv[0] === "opencode") {
    await runOpenCodeNativeTui(argv.slice(1))
    return
  }
  if (argv[0] === "claude") {
    await runClaudeNativeTui(argv.slice(1))
    return
  }
  if (argv[0] === "codex") {
    await runCodexNativeTui(argv.slice(1))
    return
  }

  transcriptParserRegistration.ensureRegistered()
  processLoggers.initialize("cli")
  getLogger("cli.main")?.info("starting cli process", { argv })
  const runtimeBootstrap = await bootstrapCliRuntime({
    argv,
    cwd: process.cwd(),
    logger: getLogger("cli.main"),
  })
  if (runtimeBootstrap.kind === "deleted_session") {
    return
  }
  await render(
    () => <ArrobaCliApp bootstrap={runtimeBootstrap.bootstrap} />,
    {
      targetFps: 60,
      gatherStats: false,
      exitOnCtrlC: false,
      useKittyKeyboard: {},
      useMouse: true,
      enableMouseMovement: false,
      useAlternateScreen: true,
      autoFocus: true,
      openConsoleOnError: OPEN_CONSOLE_ON_ERROR,
    },
  )
  getLogger("cli.main")?.info("render mounted")
}

function ArrobaCliApp(props: { bootstrap: BootstrapState }) {
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

  const isAttached = () => attachmentState() !== null
  const focusedAgentId = () => focusedAgentIdForSession(sessionState())
  const responsePaneProjectionController = createResponsePaneProjectionController({
    isAttached,
    getSession: sessionState,
    getFocusedAgentId: focusedAgentId,
    getWorkspaceScreenMode: workspaceScreenMode,
    getResponseLayout: multiAgentResponseLayout,
    getMaxAgentsPerScreen: maxAgentsPerScreen,
    workflowScreenActive: () => workflowScreenActive(),
  })
  const multiAgentMode = responsePaneProjectionController.multiAgentMode
  const workflowScreenShowing = responsePaneProjectionController.workflowScreenShowing
  const splitAgentResponseMode = responsePaneProjectionController.splitAgentResponseMode
  const interactionProjectionController = createInteractionProjectionController({
    getSession: sessionState,
    getFocusedAgentId: focusedAgentId,
  })
  const activeInteractionForAgent = interactionProjectionController.activeInteractionForAgent
  const focusedAgentInteraction = interactionProjectionController.focusedAgentInteraction
  const workflowPromptState = createMemo(() => deriveWorkflowPromptState({
    workflowScreenActive: workflowScreenShowing(),
    workflows: sessionState().workflows ?? [],
    workflowRuns: sessionState().workflow_runs ?? [],
    selectedWorkflowId: selectedWorkflowId(),
    selectedWorkflowNodeId: selectedWorkflowNodeId(),
  }))
  const responsePaneSelection = responsePaneProjectionController.responsePaneSelection
  const responsePaneAgentSignature = responsePaneProjectionController.responsePaneAgentSignature
  const responsePrimaryAgent = responsePaneProjectionController.responsePrimaryAgent
  const responseVisibleAgents = responsePaneProjectionController.responseVisibleAgents
  const visibleTranscriptAgentId = responsePaneProjectionController.visibleTranscriptAgentId
  const responsePaneRows = responsePaneProjectionController.responsePaneRows
  createEffect(() => {
    if (!isAttached()) {
      return
    }
    const session = sessionState()
    setWorkspaceShellContext((previous) =>
      deriveWorkspaceShellContextForSession(previous, session, attachmentState()?.id))
  })
  const primaryTranscriptSurfaceTone = responsePaneProjectionController.primaryTranscriptSurfaceTone
  const auxiliaryTranscriptSurfaceTone = responsePaneProjectionController.auxiliaryTranscriptSurfaceTone
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
  const agentRuntimeProjectionController = createAgentRuntimeProjectionController({
    getSession: sessionState,
    getFocusedAgentId: focusedAgentId,
    getProviderRun: providerRunState,
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
    getActiveToolLabels: primaryTranscriptRuntimeStore.activeToolLabelValues,
    getAgentPaneToolUpdates: agentPaneRuntimeStore.toolUpdatesForAgent,
    getAgentPanePreviews: agentPanePreviews,
    getAgentActivityLabels: agentActivityLabels,
    updateAgentActivityLabels: (updater) => {
      setAgentActivityLabels((current) => updater(current))
    },
    getAgentBusyLatches: agentBusyLatches,
    updateAgentBusyLatches: (updater) => {
      setAgentBusyLatches((current) => updater(current))
    },
    getSubmitting: submitting,
    getSubmittingAgentId: promptSubmissionAgentStateController.getSubmittingAgentId,
    getStreamingAgentId: streamingAgentId,
  })
  const agentPanePreview = agentRuntimeProjectionController.agentPanePreview
  const agentActivityLabel = agentRuntimeProjectionController.agentActivityLabel
  const focusedAgent = agentRuntimeProjectionController.focusedAgent
  const focusedBackendProvider = agentRuntimeProjectionController.focusedBackendProvider
  const focusedProviderRun = agentRuntimeProjectionController.focusedProviderRun
  const resolveSessionAgent = agentRuntimeProjectionController.resolveSessionAgent
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
  const promptStateForAgent = agentRuntimeProjectionController.promptStateForAgent
  const agentQueuedDepth = agentRuntimeProjectionController.agentQueuedDepth
  const agentActivePrompt = agentRuntimeProjectionController.agentActivePrompt
  const agentBusyLatch = agentRuntimeProjectionController.agentBusyLatch
  const anyPromptWork = agentRuntimeProjectionController.anyPromptWork
  const hasPromptWorkByAgent = agentRuntimeProjectionController.hasPromptWorkByAgent
  const focusedPromptState = agentRuntimeProjectionController.focusedPromptState
  const focusedQueueDepth = agentRuntimeProjectionController.focusedQueueDepth
  const focusedActivePrompt = agentRuntimeProjectionController.focusedActivePrompt
  const activeToolLabelForAgent = agentRuntimeProjectionController.activeToolLabelForAgent
  const focusedActivityLabel = agentRuntimeProjectionController.focusedActivityLabel
  const markAgentBusy = agentRuntimeProjectionController.markAgentBusy
  const clearAgentBusy = agentRuntimeProjectionController.clearAgentBusy
  const focusedAgentBusy = agentRuntimeProjectionController.focusedAgentBusy
  const allAgentsBusyState = agentRuntimeProjectionController.allAgentsBusyState
  const setAgentActivityLabel = agentRuntimeProjectionController.setAgentActivityLabel
  const transcriptEntryProjectionController = createTranscriptEntryProjectionController({
    getEntries: () => entries,
  })
  const visibleTranscriptEntries = transcriptEntryProjectionController.visibleEntries
  const queueDepth = () => focusedQueueDepth()
  const connectedClientCount = () => sessionState().attachment_ids.length
  const activePrompt = () => focusedActivePrompt()
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
  const focusedStatusBadgeController = createFocusedStatusBadgeController({
    isAttached,
    daemonDisconnected,
    activeStatusLabel: focusedActivityLabel,
    focusedBusy: focusedAgentBusy,
    agents: allAgentsBusyState,
  })
  const focusedStatusBadge = focusedStatusBadgeController.badge
  const runtimeDebugLogger = createCliRuntimeDebugLogger({
    logger: appLogger,
    debugLogsEnabled: DEBUG_LOGS_ENABLED,
    getResponseLayout: multiAgentResponseLayout,
    splitAgentResponseMode,
    isAttached,
    getAgentCount: () => sessionState().agents.length,
    getFocusedAgentId: focusedAgentId,
    hasTranscriptScrollbox: transcriptScrollboxRefController.hasScrollbox,
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
  })
  const logProviderRunDebug = runtimeDebugLogger.logProviderRun
  const logViewDebug = runtimeDebugLogger.logView
  const logVisibleTranscriptOutput = runtimeDebugLogger.logVisibleTranscriptOutput
  const logFocusedBadgeChange = runtimeDebugLogger.logFocusedBadgeChange
  createEffect(() => {
    logViewDebug("state changed")
  })
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
  const turnCompletionController = createTurnCompletionController({
    now: Date.now,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    hasActivePrompt: () => Boolean(activePrompt()),
    getDelayMs: (lastTurnActivityAt) => getTurnCompletionDelayMs({
      sessionHasPromptWork: sessionHasPromptWork(sessionState()),
      pendingTerminalRecordCount: terminalOutputRecordQueue.pendingCount(),
      pendingTerminalRecordFlush: terminalOutputRecordQueue.hasPendingFlush(),
      lastTurnActivityAt,
      now: Date.now(),
      quietWindowMs: TURN_COMPLETION_QUIET_MS,
    }),
    completeTurn: () => {
      batch(() => {
        primaryTranscriptRuntimeStore.clearActiveToolLabels()
        setAgentActivityLabels({})
        setStreamingAgentId(null)
        setSubmitting(false)
        promptSubmissionAgentStateController.clearSubmittingAgentId()
        setAgentBusyLatches({})
        setProviderActivityLabel(null)
        setActiveStatusLabel(null)
        if (!activePrompt() && statusLine() === "Cancellation requested.") {
          setStatusLine(DEFAULT_CONNECTED_STATUS)
        }
        setWorking(false)
      })
      renderSessionChromeBoundary()
    },
  })
  const cancelPendingTurnCompletion = turnCompletionController.cancelPending
  const recordTurnActivity = (_activityType: string) => {
    turnCompletionController.recordActivity()
  }
  const maybeScheduleConfirmedTurnCompletion = turnCompletionController.maybeScheduleConfirmed
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
  const expandedTurnIdsForAgent = (agentId: string | null | undefined) => agentId ? (expandedTurnIdsByAgent()[agentId] ?? []) : []
  const transcriptTurnExpansionController = createTranscriptTurnExpansionController({
    expandedTurnIdsForAgent,
    updateExpandedTurnIdsByAgent: (updater) => {
      setExpandedTurnIdsByAgent((current) => updater(current))
    },
  })
  const setExpandedTurnState = transcriptTurnExpansionController.setExpandedTurnState
  const replaceExpandedTurnsForAgent = transcriptTurnExpansionController.replaceExpandedTurnsForAgent
  const collapseLatestTurnForAgent = transcriptTurnExpansionController.collapseLatestTurnForAgent
  const applyExpandedTurns = transcriptTurnExpansionController.applyExpandedTurns

  const transcriptStateController = createTranscriptStateController({
    entries: transcriptEntryProjectionController.renderableEntries,
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    entryCounter,
    setEntryCounter,
    currentTurnId: transcriptTurnStateController.getCurrentTurnId,
    visibleTranscriptAgentId,
    expandedTurnIdsForAgent,
    setExpandedTurnState: (agentId, turnId, expanded) => {
      setExpandedTurnState(agentId, turnId, expanded)
    },
    persistVisibleTranscriptEntries: (nextEntries) => {
      persistVisibleTranscriptEntries(nextEntries)
    },
    reconcileMountedTranscript: (currentEntries, nextEntries) => {
      reconcileMountedTranscript(currentEntries, nextEntries)
    },
    retainPromptFocus,
    enforceTranscriptRetention: () => enforceTranscriptRetention(),
  })
  const applyVisibleTranscriptState = transcriptStateController.applyVisibleState
  const toggleTurn = transcriptStateController.toggleTurn
  const toggleBlob = transcriptStateController.toggleBlob
  const appendEntry = transcriptStateController.appendEntry

  const transcriptViewportController = createTranscriptViewportController({
    getScrollbox: transcriptScrollboxRefController.current,
    cancelHistoryScrollRestore: () => historyScrollRestoreController.cancel(),
    setLastTranscriptScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
  })
  const scrollTranscriptToBottom = transcriptViewportController.scrollToBottom

  const trackAgentFocusTransition = <T,>(operation: () => Promise<T>): Promise<T> =>
    agentFocusTransitionController.track(operation)

  const waitForPendingAgentFocusTransition = (): Promise<void> =>
    agentFocusTransitionController.wait()

  const transcriptEventController = createTranscriptEventController({
    recordTurnActivity,
    resetTurnCompletion: () => turnCompletionController.reset(),
    cancelPendingTurnCompletion,
    focusedAgentId,
    visibleTranscriptAgentId,
    responsePrimaryAgent,
    splitAgentResponseMode,
    isAttached,
    entries: () => entries,
    nextTurnId: transcriptTurnStateController.getNextTurnId,
    setNextTurnId: transcriptTurnStateController.setNextTurnId,
    setCurrentTurnId: transcriptTurnStateController.setCurrentTurnId,
    setSubmittingAgentId: promptSubmissionAgentStateController.setSubmittingAgentId,
    setStreamingAgentId,
    markAgentBusy,
    clearAgentBusy,
    currentAgentPaneEntries: (agentId) => currentAgentPaneEntries(agentId),
    collapseLatestTurnForAgent: (agentId, paneEntries) => collapseLatestTurnForAgent(agentId, paneEntries),
    appendTranscriptEntryToAgentPane: (agentId, entry, turnIds) => {
      appendTranscriptEntryToAgentPane(agentId, entry, turnIds ? [...turnIds] : undefined)
    },
    appendEntry,
    setSubmitting,
    setWorking,
    renderSessionChromeBoundary: () => renderSessionChromeBoundary(),
    syncVisibleTranscriptPreview: () => syncVisibleTranscriptPreview(),
    scrollTranscriptToBottom,
    updateSessionChrome: () => updateSessionChrome(),
    setWaitingRoomCloudNotice,
    rebuildTranscript: () => rebuildTranscript(),
  })
  const appendUserPrompt = transcriptEventController.appendUserPrompt
  const appendNotice = transcriptEventController.appendNotice
  const appendCloudNotice = transcriptEventController.appendCloudNotice
  const appendProviderError = transcriptEventController.appendProviderError

  const transcriptRetentionController = createTranscriptRetentionController({
    entries: () => entries.slice(),
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    renderables: primaryTranscriptRuntimeStore.transcriptRenderables,
    removeFromScrollbox: (renderableId) => {
      return transcriptScrollboxRefController.remove(renderableId)
    },
    requestScrollboxRender: transcriptScrollboxRefController.requestRender,
    deleteTool: primaryTranscriptRuntimeStore.deleteTool,
    maxEntries: LIVE_TRANSCRIPT_LIMIT,
    maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
  })
  const removeTranscriptRenderable = transcriptRetentionController.removeRenderable
  const enforceTranscriptRetention = transcriptRetentionController.enforce

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

  const authoritativeIdleController = createAuthoritativeIdleController({
    batchUpdate: batch,
    resetTurnCompletion: turnCompletionController.reset,
    clearActiveToolLabels: primaryTranscriptRuntimeStore.clearActiveToolLabels,
    setAgentActivityLabels,
    setStreamingAgentId,
    setSubmitting,
    clearSubmittingAgentId: promptSubmissionAgentStateController.clearSubmittingAgentId,
    resetPromptStop: promptStopController.reset,
    setAgentBusyLatches,
    setProviderActivityLabel,
    setActiveStatusLabel,
    setWorking,
    getStatusLine: statusLine,
    setStatusLine,
    renderSessionChromeBoundary: () => {
      renderSessionChromeBoundary()
    },
  })
  const clearLocalBusyStateForAuthoritativeIdle = authoritativeIdleController.clear

  const providerActivityController = createProviderActivityController({
    setWorking,
    handleProviderActivity: turnCompletionController.handleProviderActivity,
    updateSessionChrome: () => updateSessionChrome(),
  })
  const applyProviderActivity = providerActivityController.apply

  const assistantMessageCompletionController = createAssistantMessageCompletionController({
    entries: transcriptEntryProjectionController.renderableEntries,
    visibleTranscriptAgentId,
    splitAgentResponseMode,
    currentAgentPaneEntries: (agentId) => currentAgentPaneEntries(agentId),
    expandedTurnIdsForAgent,
    setExpandedTurnIdsForAgent: (agentId, turnIds) => {
      setExpandedTurnIdsByAgent((current) => ({
        ...current,
        [agentId]: turnIds,
      }))
    },
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    setEntryCounter,
    persistVisibleTranscriptEntries: (nextEntries) => {
      persistVisibleTranscriptEntries(nextEntries)
    },
    reconcileMountedTranscript: (currentEntries, nextEntries) => {
      reconcileMountedTranscript(currentEntries, nextEntries)
    },
    setAgentTranscriptEntries: (agentId, nextEntries) => {
      setAgentTranscriptEntries(agentId, nextEntries)
    },
    clearAgentBusy,
    confirmTurnCompletion: turnCompletionController.confirm,
    maybeScheduleConfirmedTurnCompletion,
  })
  const markAssistantMessageCompleted = assistantMessageCompletionController.markCompleted

  const visibleActivityLabelController = createVisibleActivityLabelController({
    focusedActivityLabel,
    setActiveStatusLabel,
  })
  const syncVisibleActivityLabel = visibleActivityLabelController.sync

  const transcriptStreamController = createTranscriptStreamController({
    entries: () => entries,
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    entryCounter,
    currentTurnId: transcriptTurnStateController.getCurrentTurnId,
    tools: primaryTranscriptRuntimeStore.tools,
    activeToolLabels: primaryTranscriptRuntimeStore.activeToolLabels,
    cancelPendingTurnCompletion,
    setWorking,
    setSubmitting,
    updateSessionChrome: () => updateSessionChrome(),
    syncVisibleActivityLabel,
    applyVisibleTranscriptState,
    persistVisibleTranscriptEntries: (nextEntries) => {
      persistVisibleTranscriptEntries(nextEntries)
    },
    reconcileMountedTranscript: (currentEntries, nextEntries) => {
      reconcileMountedTranscript(currentEntries, nextEntries)
    },
    updateTranscriptEntry: (entryId, text, sourceText) => {
      updateTranscriptEntry(entryId, text, sourceText)
    },
    logVisibleTranscriptOutput,
    enforceTranscriptRetention,
    maybeScheduleConfirmedTurnCompletion,
  })
  const appendProviderChunk = transcriptStreamController.appendProviderChunk
  const appendToolUpdate = transcriptStreamController.appendToolUpdate

  let processKernelTerminalOutputRecord: (record: TerminalOutputRecord) => void = () => {}
  const terminalOutputRecordProcessor = createTerminalOutputRecordProcessor({
    appendPromptEchoToSharedHistory,
    processKernelTerminalOutputRecord: (record) => {
      processKernelTerminalOutputRecord(record)
    },
  })
  const processTerminalOutputRecord = terminalOutputRecordProcessor.process

  const terminalOutputRecordQueue = createTerminalOutputRecordQueue<ReturnType<typeof startTimeout>, TerminalOutputRecord>({
    delayMs: STREAM_BATCH_WINDOW_MS,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    processRecords(records) {
      runUiBatch(() => {
        for (const record of records) {
          processTerminalOutputRecord(record)
        }
      })
    },
  })
  const flushPendingTerminalRecords = terminalOutputRecordQueue.flush
  const queueTerminalOutputRecords = terminalOutputRecordQueue.queue

  const splitPaneFooterRenderController = createSplitPaneFooterRenderController({
    renderer,
    state: splitPaneFooterRenderState,
    primaryBox: responsePaneRenderRefStore.getPrimaryFooterBox,
    auxiliaryBoxes: responsePaneRenderRefStore.getAuxiliaryFooterBoxes,
    isAttached,
    workflowScreenActive: () => workflowScreenActive(),
    maxAgentsPerScreen,
    visibleAgents: responseVisibleAgents,
    focusedAgentId,
    providerRun: providerRunState,
    currentProviderSelection,
    agentActivityLabels,
    hasPromptWorkByAgent,
    streamingAgentId,
    agentBusyLatch,
    sessionConfigValues: () => sessionState().config_state?.values,
    agentLocationLabel,
    badgeWidth: STATUS_BADGE_WIDTH,
    animationFrame: workingAnimationFrame,
    renderFooters: renderSplitPaneFootersView,
  })
  const renderSplitPaneFooters = splitPaneFooterRenderController.render

  const agentInteractionStripController = createAgentInteractionStripController({
    renderer,
    primaryBox: responsePaneRenderRefStore.getPrimaryInteractionBox,
    auxiliaryBoxes: responsePaneRenderRefStore.getAuxiliaryInteractionBoxes,
    visibleAgents: responseVisibleAgents,
    maxAgentsPerScreen,
    focusedAgentId,
    activeInteractionForAgent,
    selectedChoiceIndex: interactionChoiceStore.selectedChoiceIndex,
    setSelectedChoiceIndex: interactionChoiceStore.setSelectedIndex,
    customReply: interactionChoiceStore.customReply,
    customEditing: interactionChoiceStore.isCustomEditing,
    renderStrips: renderAgentInteractionStrips,
  })
  const renderAgentInteractions = agentInteractionStripController.render

  const promptMetaRenderController = createPromptMetaRenderController({
    getUsage: promptUsageMeta,
    onRefAssigned: () => {
      updateSessionChrome()
    },
    renderMeta: renderPromptMeta,
  })
  const setPromptMetaRenderables = promptMetaRenderController.setRenderables
  const assignPromptMetaRef = promptMetaRenderController.assignRefCallback

  const requestTranscriptRender = () => {
    transcriptRenderDeferralController.request()
  }

  const loadingStateController = createCliLoadingStateController({
    getSessionHydrating: sessionHydrating,
    setSessionHydrating,
    setLoadingHistory,
    renderHistoryLoadingIndicator,
    isAttached,
    visibleTranscriptEntryCount: transcriptEntryProjectionController.visibleEntryCount,
    workflowScreenActive: () => workflowScreenActive(),
    rebuildTranscript: () => rebuildTranscript(),
    requestTranscriptRender,
  })
  const setHistoryLoadingState = loadingStateController.setHistoryLoadingState
  const setSessionHydratingState = loadingStateController.setSessionHydratingState

  const runUiBatch = uiBatchController.run

  const statusIndicatorController = createStatusIndicatorController<BoxRenderable>({
    isAttached,
    getBadge: focusedStatusBadge,
    getAnimationFrame: workingAnimationFrame,
    resetFocusedBadgeChange: runtimeDebugLogger.resetFocusedBadgeChange,
    logFocusedBadgeChange,
    renderIndicator: ({ box, attached, badge, animationFrame }) => {
      renderStatusIndicatorView({
        renderer,
        box,
        state: statusIndicatorRenderState,
        attached,
        badge,
        badgeWidth: STATUS_BADGE_WIDTH,
        animationFrame,
      })
    },
  })
  const renderStatusIndicator = statusIndicatorController.render

  const responseLayoutController = createResponseLayoutController({
    getRefs: () => responsePaneRenderRefStore.snapshot({
      primaryScrollbox: transcriptScrollboxRefController.current(),
      historyLoadingBox: historyLoadingRenderController.getBox(),
    }),
    getSplit: splitAgentResponseMode,
    getVisibleAgents: responseVisibleAgents,
    getPaneRows: responsePaneRows,
    getFocusedAgentId: focusedAgentId,
    getShowWorkflowScreen: () => workflowScreenActive(),
    getMaxAgentsPerScreen: maxAgentsPerScreen,
    getResponsePaneSelection: responsePaneSelection,
    getTheme: () => theme,
    emptyTextAttributes: TextAttributes.NONE,
    panelBackgroundForFocus: (focused) => transcriptSurfacePalette(resolveTranscriptSurfaceTone(true, focused)).panel,
    renderSplitPaneFooters,
    renderAgentInteractions,
    clearAuxiliaryAgentPane: (agentId) => {
      clearAuxiliaryAgentPane(agentId)
    },
    unregisterAgentScrollbox: agentPaneRuntimeStore.unregisterScrollbox,
    getCurrentAuxiliaryAgentId: agentPaneRuntimeStore.getCurrentAuxiliaryAgentId,
    setCurrentAuxiliaryAgentId: agentPaneRuntimeStore.setCurrentAuxiliaryAgentId,
    registerAgentScrollbox: agentPaneRuntimeStore.registerScrollbox,
    rebuildAuxiliaryAgentPane: (agentId) => {
      rebuildAuxiliaryAgentPane(agentId)
    },
    buildEmptyTranscriptRenderable: () => buildEmptyTranscriptRenderable(renderer),
    getMountedTranscriptAgentId: primaryTranscriptRuntimeStore.getMountedTranscriptAgentId,
    getAgentPaneEntries: (agentId) => agentPaneEntries()[agentId] ?? [],
    replaceTranscriptEntries: (nextEntries, agentId) => {
      replaceTranscriptEntries(nextEntries, agentId)
    },
    scheduleResponsePaneRepaint,
    logViewDebug,
  })
  const applyResponseLayout = responseLayoutController.apply

  createEffect(() => {
    splitAgentResponseMode()
    workspaceScreenMode()
    multiAgentResponseLayout()
    maxAgentsPerScreen()
    dimensions().width
    responsePaneAgentSignature()
    focusedAgentId()
    applyResponseLayout()
  })

  createEffect(() => {
    if (isAttached()) {
      return
    }
    promptSubmissionAgentStateController.clearSubmittingAgentId()
    setAgentBusyLatches({})
  })

  createEffect(() => {
    providerRunState()?.model
    providerRunState()?.variant
    working()
    activeStatusLabel()
    providerActivityLabel()
    streamingAgentId()
    agentBusyLatches()
    for (const agent of sessionState().agents) {
      agent.is_processing
      agent.state
    }
    agentActivityLabels()
    updateSessionChrome()
  })

  const responsePaneRepaintController = createResponsePaneRepaintController({
    scheduleTimer: startTimeout,
    repaint: () => {
      applyResponseLayout()
      scheduleResponsePaneRepaint()
    },
  })
  const refreshSplitPaneFocusRepaint = responsePaneRepaintController.refreshFocus

  const sessionChromeRenderController = createSessionChromeRenderController({
    renderer,
    createSummaryRenderState: createSessionChromeSummaryRenderState,
    renderSummary: (options) => {
      renderSessionChromeSummary(options as unknown as Parameters<typeof renderSessionChromeSummary>[0])
    },
    syncPromptPlaceholder,
    getFatalError: fatalError,
    getSubmitting: submitting,
    getFooterHint: footerHint,
    isAttached,
    getSession: sessionState,
    getConnectedClientCount: connectedClientCount,
    getMultiAgentMode: multiAgentMode,
    getResponseLayout: multiAgentResponseLayout,
    getSessionStatusMode: sessionStatusMode,
    getFocusedHasPromptWork: () => agentHasPromptWork(sessionState(), focusedAgentId()),
    getHotkeyToggleLabel: () => HOTKEY_TOGGLE_LABEL,
    getFooterFlash: footerFlash,
    getPromptMetaParts: promptMetaParts,
    setPromptMetaRenderables,
    renderStatusIndicator,
    renderSplitPaneFooters,
    renderAgentInteractions,
    getWorking: working,
    getActiveStatusLabel: activeStatusLabel,
    getProviderActivityLabel: providerActivityLabel,
    getStreamingAgentId: streamingAgentId,
  })

  sessionChromeUpdateController = createSessionChromeUpdateController({
    delayMs: CHROME_UPDATE_THROTTLE_MS,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    isBatched: uiBatchController.isBatched,
    applyUpdate: sessionChromeRenderController.apply,
  })
  const renderSessionChromeBoundary = sessionChromeUpdateController.flush
  const updateSessionChrome = () => {
    sessionChromeUpdateController.request(sessionChromeRenderController.shouldThrottle())
  }

  const agentPaneStoreController = createAgentPaneStoreController({
    isAttached,
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
    getVisibleTranscriptEntries: transcriptEntryProjectionController.renderableEntries,
    getPaneEntriesByAgent: agentPaneEntries,
    updatePaneEntries: (updater) => {
      setAgentPaneEntries((current) => updater(current))
    },
    updatePanePreviews: (updater) => {
      setAgentPanePreviews((current) => updater(current))
    },
    getSessionAgents: () => sessionState().agents,
    getFocusedAgentId: focusedAgentId,
    getMaxAgentsPerScreen: maxAgentsPerScreen,
    splitAgentResponseMode,
    getPrimaryAgentId: () => responsePrimaryAgent()?.id ?? null,
    expandedTurnIdsForAgent,
    replaceTranscriptEntries: (nextEntries, agentId) => {
      replaceTranscriptEntries(nextEntries, agentId)
    },
    reconcileMountedAuxiliaryTranscript: (agentId, previousPaneEntries, sanitizedEntries) => {
      reconcileMountedAuxiliaryTranscript(agentId, previousPaneEntries, sanitizedEntries)
    },
  })
  const setAgentPanePreview = agentPaneStoreController.setAgentPanePreview
  const persistVisibleTranscriptEntries = agentPaneStoreController.persistVisibleTranscriptEntries
  const setAgentTranscriptEntries = agentPaneStoreController.setAgentTranscriptEntries
  const visibleAuxiliaryAgentIds = agentPaneStoreController.visibleAuxiliaryAgentIds
  const commitAgentPaneEntries = agentPaneStoreController.commitAgentPaneEntries
  const currentAgentPaneEntries = agentPaneStoreController.currentAgentPaneEntries

  const agentPaneTranscriptEntryController = createAgentPaneTranscriptEntryController({
    currentAgentPaneEntries,
    visibleTranscriptAgentId,
    visibleTranscriptEntries: transcriptEntryProjectionController.renderableEntries,
    expandedTurnIdsForAgent,
    setAgentPanePreview,
    updateAgentPanePreviews: (updater) => {
      setAgentPanePreviews((current) => updater(current))
    },
    trimLiveAgentPaneEntries: (agentId, nextEntries) => trimLiveAgentPaneEntries(agentId, nextEntries),
    setAgentTranscriptEntries: (agentId, nextEntries, turnIds) => {
      setAgentTranscriptEntries(agentId, nextEntries, turnIds ? [...turnIds] : undefined)
    },
  })
  const hasTrailingUserPrompt = agentPaneTranscriptEntryController.hasTrailingUserPrompt

  const agentPaneTranscriptInteractionController = createAgentPaneTranscriptInteractionController({
    currentAgentPaneEntries,
    expandedTurnIdsForAgent,
    setExpandedTurnState,
    commitAgentPaneEntries: (agentId, nextEntries) => {
      commitAgentPaneEntries(agentId, nextEntries)
    },
    reconcileMountedAuxiliaryTranscript: (agentId, currentEntries, nextEntries) => {
      reconcileMountedAuxiliaryTranscript(agentId, currentEntries, nextEntries)
    },
    retainPromptFocus,
  })
  const toggleAuxiliaryPaneTurn = agentPaneTranscriptInteractionController.toggleTurn
  const toggleAuxiliaryPaneBlob = agentPaneTranscriptInteractionController.toggleBlob

  const agentPaneTranscriptRenderController = createAgentPaneTranscriptRenderController({
    scrollboxes: agentPaneRuntimeStore.scrollboxes,
    entryRenderables: agentPaneRuntimeStore.entryRenderables,
    emptyRenderables: agentPaneRuntimeStore.emptyRenderables,
    toolStates: agentPaneRuntimeStore.toolStates,
    paneEntries: (agentId) => agentPaneEntries()[agentId] ?? [],
    buildEmptyRenderable: () => buildEmptyTranscriptRenderable(renderer),
    buildEntryRenderable: (agentId, entry) => buildTranscriptEntryRenderable(
      renderer,
      entry,
      transcriptSyntaxStyleController.current(),
      (turnId, nextToggleEntryId) => toggleAuxiliaryPaneTurn(agentId, turnId, nextToggleEntryId),
      (entryId, collapsed) => toggleAuxiliaryPaneBlob(agentId, entryId, collapsed),
      auxiliaryTranscriptSurfaceTone(agentId),
    ),
    renderMode: transcriptRenderMode,
    requestRenderable: (renderable) => renderScheduler.requestRenderable(renderable),
    clampScrollTop,
    activeAgentIdsForSession: (session: RuntimeSession) => splitPaneAuxiliaryAgentIds(
      session.agents,
      session.focused_agent_id,
      true,
      maxAgentsPerScreen(),
    ),
  })
  const auxiliaryAgentPaneTools = agentPaneTranscriptRenderController.toolStateForAgent
  const clearAuxiliaryAgentPane = agentPaneTranscriptRenderController.clearPane
  const rebuildAuxiliaryAgentPane = agentPaneTranscriptRenderController.rebuildPane
  const updateAuxiliaryTranscriptEntry = agentPaneTranscriptRenderController.updateEntry
  const reconcileMountedAuxiliaryTranscript = agentPaneTranscriptRenderController.reconcileMountedTranscript
  const pruneAuxiliaryAgentPanes = agentPaneTranscriptRenderController.prunePanes

  const agentPaneTranscriptRetentionController = createAgentPaneTranscriptRetentionController({
    maxEntries: LIVE_TRANSCRIPT_LIMIT,
    maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
    deleteToolForMergeKey: (agentId, mergeKey) => {
      auxiliaryAgentPaneTools(agentId).delete(mergeKey)
    },
  })
  const trimLiveAgentPaneEntries = agentPaneTranscriptRetentionController.trimLiveEntries

  const agentPaneStreamingCommitController = createAgentPaneStreamingCommitController({
    trimLiveAgentPaneEntries,
    expandedTurnIdsForAgent,
    commitAgentPaneEntries,
    splitAgentResponseMode,
    getResponsePrimaryAgentId: () => responsePrimaryAgent()?.id ?? null,
    replaceTranscriptEntries: (nextEntries, agentId) => {
      replaceTranscriptEntries(nextEntries, agentId)
    },
    visibleAuxiliaryAgentIds,
    updateAuxiliaryTranscriptEntry,
    reconcileMountedAuxiliaryTranscript,
  })
  const commitStreamingAgentPaneEntry = agentPaneStreamingCommitController.commitStreamingEntry

  const syncVisibleTranscriptPreview = agentPaneTranscriptEntryController.syncVisibleTranscriptPreview
  const appendAgentPanePreview = agentPaneTranscriptEntryController.appendPreview
  const appendTranscriptEntryToAgentPane = agentPaneTranscriptEntryController.appendEntry

  const agentPaneTranscriptStreamController = createAgentPaneTranscriptStreamController({
    currentAgentPaneEntries,
    trimLiveAgentPaneEntries,
    setAgentTranscriptEntries,
    commitStreamingAgentPaneEntry,
    toolStateForAgent: auxiliaryAgentPaneTools,
  })
  const appendProviderChunkToAgentPane = agentPaneTranscriptStreamController.appendProviderChunk
  const appendToolUpdateToAgentPane = agentPaneTranscriptStreamController.appendToolUpdate

  createEffect(() => {
    if (!isAttached()) {
      return
    }
    const agentId = responsePrimaryAgent()?.id ?? null
    const currentEntries = transcriptEntryProjectionController.renderableEntries().map((entry) => ({ ...entry }))
    if (!agentId || agentId !== primaryTranscriptRuntimeStore.getMountedTranscriptAgentId()) {
      return
    }
    setAgentPaneEntries((current) => ({
      ...current,
      [agentId]: currentEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(currentEntries))
  })

  const agentPaneRefreshController = createAgentPaneRefreshController({
    getCurrentAgents: () => sessionState().agents,
    getFocusedAgentId: focusedAgentId,
    getExpandedTurnIdsByAgent: expandedTurnIdsByAgent,
    currentAgentPaneEntries,
    splitAgentResponseMode,
    maxAgentsPerScreen,
    loadHistoryPage: async (sessionId, agentId, cursor) => {
      const historyPage = await getSessionHistory(client, sessionId, cursor, agentId)
      return {
        entries: historyPage.entries,
        nextCursor: historyPage.next_cursor,
      }
    },
    pruneAuxiliaryAgentPanes,
    setExpandedTurnIdsByAgent,
    setAgentPanePreviews,
    setAgentPaneEntries,
    setNextHistoryCursor,
    applyExpandedTurns,
    replaceTranscriptEntries: (entries, agentId) => replaceTranscriptEntries(entries, agentId),
    applyResponseLayout,
    rebuildAuxiliaryAgentPane,
  })
  const refreshAgentPanes = agentPaneRefreshController.refresh
  const shouldRefreshAgentPanesForSessionChange = agentPaneRefreshController.shouldRefreshForSessionChange

  const primaryTranscriptRenderController = createPrimaryTranscriptRenderController({
    getScrollbox: transcriptScrollboxRefController.current,
    getEmptyRenderable: primaryTranscriptRuntimeStore.getEmptyRenderable,
    setEmptyRenderable: primaryTranscriptRuntimeStore.setEmptyRenderable,
    renderables: primaryTranscriptRuntimeStore.transcriptRenderables,
    visibleEntries: visibleTranscriptEntries,
    workflowScreenActive: () => workflowScreenActive(),
    showWorkflowOutline: () => isAttached() && workflowScreenActive(),
    buildWorkflowRenderable: () => buildWorkflowOutlineRenderable(renderer, {
      workflows: sessionState().workflows ?? [],
      agents: sessionState().agents,
      workflowRuns: sessionState().workflow_runs ?? [],
      selectedWorkflowId: selectedWorkflowId(),
      selectedNodeId: selectedWorkflowNodeId(),
      onSelectNode: (nodeId) => {
        setSelectedWorkflowNodeId(nodeId)
        rebuildTranscript()
      },
      inspector: workflowInspector(),
      shellPane: {
        entries: workspaceShellEntries(),
        sessionId: workspaceShellContext().sessionId ?? null,
        agentId: workspaceShellContext().agentId ?? null,
      },
    }),
    buildEmptyRenderable: () => isAttached()
      ? (sessionHydrating()
          ? buildLoadingTranscriptRenderable(renderer)
          : buildEmptyTranscriptRenderable(renderer))
      : buildNoSessionRenderable(renderer, waitingRoomState(), availableSessions(), providerCatalogState(), {
        cloudNotice: waitingRoomCloudNotice(),
        inventoryStatus: waitingRoomInventoryStatus(),
        loadingFrame: waitingRoomState().introStep,
        relay: relayStatusState(),
        machines: remoteMachinesState(),
        kernels: remoteKernelsState(),
        terminals: terminalsState(),
      }, waitingRoomTargets(), themeRegistryState()),
    buildEntryRenderable: (entry) => buildTranscriptEntryRenderable(
      renderer,
      entry,
      transcriptSyntaxStyleController.current(),
      toggleTurn,
      toggleBlob,
      primaryTranscriptSurfaceTone(),
    ),
    renderMode: transcriptRenderMode,
    requestTranscriptRender,
    requestRendererRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
    shouldResetEmptyScrollTop: isAttached,
    clampScrollTop,
    setLastScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
    logViewDebug,
  })
  const mountTranscriptEntry = primaryTranscriptRenderController.mountEntry
  const reconcileMountedTranscript = primaryTranscriptRenderController.reconcileMountedTranscript
  const updateTranscriptEntry = primaryTranscriptRenderController.updateEntry
  const rebuildTranscript = primaryTranscriptRenderController.rebuildTranscript

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

  const primaryTranscriptEntryController = createPrimaryTranscriptEntryController({
    getScrollbox: transcriptScrollboxRefController.current,
    getEntries: transcriptEntryProjectionController.renderableEntries,
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
    expandedTurnIdsForAgent,
    clearToolState: primaryTranscriptRuntimeStore.clearTools,
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    setEntryCounter,
    setCurrentTurnId: transcriptTurnStateController.setCurrentTurnId,
    setNextTurnId: transcriptTurnStateController.setNextTurnId,
    setMountedTranscriptAgentId: primaryTranscriptRuntimeStore.setMountedTranscriptAgentId,
    setLastScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
    rebuildTranscript,
    syncVisibleTranscriptPreview,
    restorePrependedHistory: (request) => historyScrollRestoreController.restorePrependedHistory(request),
  })
  const replaceTranscriptEntries = primaryTranscriptEntryController.replaceEntries
  const prependTranscriptEntries = primaryTranscriptEntryController.prependEntries

  const agentPaneRuntimeResetController = createAgentPaneRuntimeResetController({
    clearRenderedPanes: agentPaneTranscriptRenderController.clearAll,
    clearCurrentAuxiliaryAgentIds: agentPaneRuntimeStore.clearCurrentAuxiliaryAgentIds,
  })
  const clearAgentPaneRuntime = agentPaneRuntimeResetController.reset

  const attachedSessionPrimeController = createAttachedSessionPrimeController({
    promptHistoryHydrationController,
    splitAgentResponseMode,
    maxAgentsPerScreen,
    loadVisibleAgentHistory: (sessionId, agentId) => getSessionHistory(client, sessionId, null, agentId),
    setAgentPaneEntries: (agentId, nextEntries) => {
      setAgentPaneEntries((current) => ({
        ...current,
        [agentId]: nextEntries,
      }))
    },
    setAgentPanePreview,
    replaceTranscriptEntries,
    setNextHistoryCursor,
  })
  const primeAttachedSessionBinding = attachedSessionPrimeController.prime

  const deferredBootstrapController = createDeferredBootstrapController({
    getDeferred: () => props.bootstrap.deferred,
    currentAttachmentSessionId: () => attachmentState()?.session_id ?? null,
    currentTranscriptEntryCount: () => transcriptEntryProjectionController.renderableEntries().length,
    entryCounter,
    setProviderCatalog: setProviderCatalogState,
    setProviderCommandCatalogs: setProviderCommandCatalogState,
    updateSessionChrome,
    setPromptHistoryEntries,
    resetPromptHistoryNavigation: () => {
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(null)
    },
    setNextHistoryCursor,
    setAgentPaneEntries: (agentId, nextEntries) => {
      setAgentPaneEntries((current) => ({
        ...current,
        [agentId]: nextEntries,
      }))
    },
    setAgentPanePreview,
    replaceTranscriptEntries,
    prependTranscriptEntries,
    logWarning: (message, fields) => {
      appLogger?.warn(message, fields)
    },
    formatError,
  })
  const applyDeferredBootstrap = deferredBootstrapController.apply

  onMount(() => {
    applyDeferredBootstrap()
  })

  const transcriptHistoryLoadController = createTranscriptHistoryLoadController({
    isAttached,
    isLoading: loadingHistory,
    getCursor: nextHistoryCursor,
    getSessionId: () => sessionState().id,
    getVisibleAgentId: visibleTranscriptAgentId,
    getEntryCounter: entryCounter,
    setLoading: setHistoryLoadingState,
    setNextCursor: setNextHistoryCursor,
    loadPage: (sessionId, cursor, agentId) => getSessionHistory(client, sessionId, cursor, agentId),
    prependEntries: prependTranscriptEntries,
    flashError: (message) => {
      flashFooter(message, "error")
    },
    logWarning: (message, fields) => {
      appLogger?.warn(message, fields)
    },
    formatError,
  })
  const transcriptHistoryAutoloadController = createTranscriptHistoryAutoloadController({
    scheduleTimer: (callback, delayMs) => {
      startTimeout(callback, delayMs)
    },
    getScrollbox: transcriptScrollboxRefController.current,
    isScrollRestoring: () => historyScrollRestoreController.isRestoring(),
    isAttached,
    isLoadingHistory: loadingHistory,
    hasMoreHistory: () => nextHistoryCursor() !== null,
    getLastScrollTop: primaryTranscriptRuntimeStore.getLastScrollTop,
    setLastScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
    loadOlderHistory: () => transcriptHistoryLoadController.loadOlderPage(),
  })

  const {
    hydrateCurrentAttachedSession,
    finalizeAttachedSessionBinding,
  } = createSessionAttachmentController({
    isAttached,
    attachmentState,
    sessionState,
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    applySessionState,
    refreshAgentPanes,
    refreshSplitPaneFocusRepaint,
    maybeResize: (sessionId) => maybeResize(client, sessionId),
    catchUpAttachedSession: (sessionId, attachmentId, session) =>
      catchUpAttachedSession(client, sessionId, attachmentId, session, appLogger),
    formatError,
    logWarning: (message, fields) => {
      appLogger?.warn(message, fields)
    },
  })

  const kernelEventSubscriptionController = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => supportsKernelEventStream,
    getAttachment: attachmentState,
    getSessionId: () => sessionState().id,
    subscribeToWaitingRoomInventory: () => client.subscribeToWaitingRoomInventory(),
    subscribeToKernelEvents: (sessionId, attachmentId) => client.subscribeToKernelEvents(sessionId, attachmentId),
    onEvaluate: (state) => {
      appLogger?.debug("evaluating kernel event subscription", {
        session_id: state.nextSessionId,
        attachment_id: state.nextAttachmentId,
        subscribed_session_id: state.sessionId,
        subscribed_attachment_id: state.attachmentId,
        subscribed_scope: state.scope,
        attached: state.attached,
      })
    },
    onWaitingRoomSubscribed: () => {
      appLogger?.info("subscribed to waiting room inventory events")
    },
    onSessionSubscribed: (sessionId, attachmentId) => {
      appLogger?.info("subscribed to kernel events", {
        session_id: sessionId,
        attachment_id: attachmentId,
      })
    },
    onWaitingRoomSubscriptionFailed: (error) => {
      appLogger?.error("waiting room inventory subscription failed", {
        error: formatError(error),
      })
      setDaemonDisconnected(true)
      setStatusLine("Waiting to reconnect to the Arroba kernel.")
      appendNotice(`Waiting room inventory subscription failed: ${formatError(error)}`, "warning")
      updateSessionChrome()
    },
    onSessionSubscriptionFailed: (sessionId, attachmentId, error) => {
      appLogger?.error("kernel event subscription failed", {
        session_id: sessionId,
        attachment_id: attachmentId,
        error: formatError(error),
      })
      setDaemonDisconnected(true)
      setStatusLine("Waiting to reconnect to the Arroba kernel.")
      appendNotice(`Kernel event subscription failed: ${formatError(error)}`, "warning")
      updateSessionChrome()
    },
  })
  const syncKernelEventSubscription = () => kernelEventSubscriptionController.sync()
  const kernelRestartRecoveryController = createKernelRestartRecoveryController({
    isClosing: closingStateController.isClosing,
    isAttached,
    isDisconnected: daemonDisconnected,
    getSessionId: () => sessionState().id,
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    attachToSession: (sessionId) => attachToSession(client, sessionId, options.clientId),
    projectSession: (session) => applyProviderRunProfileToSession(session, providerRunState()),
    applyAttachment: setAttachmentState,
    applySession: applySessionState,
    resetKernelEventSubscription: kernelEventSubscriptionController.reset,
    syncKernelEventSubscription,
    refreshAgentPanes: () => refreshAgentPanes(sessionState()),
    clearLocalBusyStateForAuthoritativeIdle: () => {
      clearLocalBusyStateForAuthoritativeIdle(sessionState())
    },
    onRecovered: () => {
      recordDaemonActivity("kernel_restart_recovered")
      setDaemonDisconnected(false)
      setStatusLine(DEFAULT_CONNECTED_STATUS)
      updateSessionChrome()
      appendNotice("Reconnected to the Arroba kernel.")
    },
    onAttemptFailed: (sessionId, error) => {
      appLogger?.debug("kernel restart recovery attempt failed", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
    sleep,
  })
  const recoverAttachedSessionAfterKernelRestart = () => kernelRestartRecoveryController.recover()

  const {
    transitionToNoSession,
    detachCurrentAttachment,
    attachBinding,
  } = createSessionLifecycleController({
    cliOptions: options,
    connectedStatus: DEFAULT_CONNECTED_STATUS,
    waitingRoomState,
    attachmentState,
    deriveDetachedCliTransitionState,
    deriveAttachedCliTransitionState,
    clearPendingPromptAttachments,
    clearActiveToolLabels: primaryTranscriptRuntimeStore.clearActiveToolLabels,
    clearWorkflows: () => {},
    clearAgentPaneRuntime,
    clearDirectoryTree: () => setDirectoryTreeState(null),
    clearTranscript: () => replaceTranscriptEntries([]),
    refreshResponseLayout: applyResponseLayout,
    resetWorkspaceScreen: () => setWorkspaceScreenMode("agents"),
    resetStopRequestInFlight: () => {
      promptStopController.reset()
    },
    bumpHistoryLoadGeneration: () => {
      transcriptHistoryLoadController.bumpGeneration()
    },
    reconcileWaitingRoom,
    refreshWaitingRoomData,
    requestRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
    clearPromptInput: () => {
      promptInputRefController.clear()
    },
    syncPromptTextSnapshot,
    blurPromptInput: () => {
      promptInputRefController.blur()
    },
    focusPromptInput: () => {
      promptInputRefController.focus()
    },
    layoutPreference: () => preferencesState().ui?.multiAgentResponseLayout ?? null,
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
    updateSessionChrome,
    refreshSplitPaneFocusRepaint,
    attachToSession: (sessionId, clientId) => attachToSession(client, sessionId, clientId),
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    launchProviderRun: (sessionId, provider, accountProfile, model, effort, targetAgentId) =>
      launchProviderRun(client, sessionId, provider, accountProfile, model, effort, targetAgentId),
    tryGetProviderRun: (providerRunId) => tryGetProviderRun(client, providerRunId, appLogger),
    setProviderCatalogState,
    getProviderCatalog: () => getProviderCatalog(client, appLogger),
    syncCliProviderSelection: ({ provider, model, effort }) => {
      options.provider = provider
      options.model = model
      options.effort = effort
      reconcileWaitingRoom({
        ...waitingRoomState(),
        providerId: normalizeBackendProviderId(provider),
        modelId: model,
        effort,
      })
    },
    primeAttachedSessionBinding,
    hydrateAttachedSessionBinding: (sessionId, attachmentId, session) =>
      finalizeAttachedSessionBinding({ sessionId, attachmentId, session }),
    setAvailableSessions,
    listSessions: () => listSessions(client),
    scheduleShortViewportHistoryCheck: () => transcriptHistoryAutoloadController.scheduleShortViewportCheck(),
    detachAttachment: (attachmentId) => detachSessionAttachment(client, attachmentId),
    syncKernelEventSubscription,
    formatError,
    logWarning: (message, fields) => {
      appLogger?.warn(message, fields)
    },
    logAttachedProviderRun: (mode, run, fields) => {
      logProviderRunDebug(
        mode === "launched"
          ? "attached session launched provider run"
          : "attached session loaded existing provider run",
        run,
        fields,
      )
    },
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

  const providerRecoveryController = createProviderRecoveryController({
    isAttached,
    getSessionId: () => sessionState().id,
    getProvider: () => options.provider ?? "opencode",
    getAccountProfile: () => options.accountProfile,
    getModel: currentModelId,
    getEffort: currentVariantId,
    getTargetAgentId: focusedAgentId,
    launchProviderRun: ({ sessionId, provider, accountProfile, model, effort, targetAgentId }) =>
      launchProviderRun(client, sessionId, provider, accountProfile, model, effort, targetAgentId),
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    projectSession: applyProviderRunProfileToSession,
    applyProviderRun: setProviderRunState,
    applySession: applySessionState,
    resizeSession: (sessionId) => maybeResize(client, sessionId),
    onRecovered: (reason) => {
      setStatusLine("Recovered provider connection.")
      updateSessionChrome()
      flashFooter(`recovered provider run after ${reason}`, "info")
    },
    onRecoveryFailed: (reason, error) => {
      appLogger?.warn("provider recovery failed", {
        reason,
        error: formatError(error),
      })
    },
  })

  const recoverProviderRun = providerRecoveryController.recover

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

  const exitController = createCliExitController({
    isClosing: closingStateController.isClosing,
    setClosing: closingStateController.setClosing,
    getCreatedSession: createdSessionState,
    getConnectedClientCount: connectedClientCount,
    getAttachment: attachmentState,
    getSessionId: () => sessionState().id,
    getPromptDraft: persistablePromptDraft,
    syncPromptTextSnapshot,
    flushPromptDraftPersist: flushPendingPromptDraftPersist,
    persistSessionPromptDraft: (sessionId, promptDraft) =>
      persistSessionPromptState(sessionId, { promptDraft }),
    shouldEndSessionOnExit: shouldEndSessionOnCliExit,
    archiveSession: async (sessionId) => {
      await archiveSessionById(client, sessionId)
    },
    detachAttachment: (attachmentId) => detachSessionAttachment(client, attachmentId),
    getCleanupDecision: getExitCleanupDecision,
    restoreTerminalAndExit: (exitCode) => restoreTerminalAndExit(exitCode),
    onForceExitAfterCleanupFailure: () => {
      appLogger?.warn("forcing cli exit after prior cleanup failure")
    },
    onExitRequested: (createdSession) => {
      appLogger?.info("requested cli exit", {
        created_session: createdSession,
      })
    },
    onPromptDraftFlushFailed: (error) => {
      appLogger?.warn("failed to flush prompt draft during exit", {
        error: formatError(error),
      })
    },
    onPromptDraftPersistFailed: (sessionId, error) => {
      appLogger?.warn("failed to persist prompt draft during exit", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
    onCleanupFailed: (decision, error) => {
      appLogger?.warn("exit cleanup failed", {
        error: formatError(error),
        will_exit: decision.exit,
      })
      appendNotice(decision.message, "warning")
      setStatusLine(decision.message)
    },
    onCleanupCompleted: () => {
      appLogger?.info("cli exit cleanup completed")
    },
  })

  const requestExit = exitController.requestExit

  const waitingRoomTransitionController = createWaitingRoomTransitionController({
    isClosing: closingStateController.isClosing,
    getCreatedSession: createdSessionState,
    getConnectedClientCount: connectedClientCount,
    getAttachment: attachmentState,
    getSessionId: () => sessionState().id,
    getPromptDraft: persistablePromptDraft,
    syncPromptTextSnapshot,
    flushPromptDraftPersist: flushPendingPromptDraftPersist,
    persistSessionPromptDraft: (sessionId, promptDraft) =>
      persistSessionPromptState(sessionId, { promptDraft }),
    shouldEndSessionOnExit: shouldEndSessionOnCliExit,
    archiveSession: async (sessionId) => {
      await archiveSessionById(client, sessionId)
    },
    detachAttachment: (attachmentId) => detachSessionAttachment(client, attachmentId),
    transitionToWaitingRoom: (message) => {
      void transitionToNoSession(message)
    },
    onWaitingRoomRequested: (createdSession) => {
      appLogger?.info("requested waiting room", {
        created_session: createdSession,
      })
    },
    onPromptDraftFlushFailed: (error) => {
      appLogger?.warn("failed to flush prompt draft during waiting-room transition", {
        error: formatError(error),
      })
    },
    onPromptDraftPersistFailed: (sessionId, error) => {
      appLogger?.warn("failed to persist prompt draft during waiting-room transition", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
    onCleanupFailed: (error) => {
      appLogger?.warn("waiting room cleanup failed", {
        error: formatError(error),
      })
      appendNotice(formatError(error), "warning")
    },
    onTransitionCompleted: () => {
      appLogger?.info("waiting room transition completed")
    },
  })

  const requestWaitingRoom = waitingRoomTransitionController.requestWaitingRoom

  const terminalExitController = createTerminalExitController({
    renderer,
    sleep,
    exitProcess: (exitCode) => process.exit(exitCode),
    onRendererDestroyFailed: (error) => {
      appLogger?.warn("renderer destroy failed during exit", {
        error: formatError(error),
      })
    },
  })

  const restoreTerminalAndExit = terminalExitController.restoreAndExit

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
    clearTerminalOutputRecordTimer: () => terminalOutputRecordQueue.clearTimer(),
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
    recordDaemonActivity,
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
  processKernelTerminalOutputRecord = runtimeProcessKernelTerminalOutputRecord

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
        statusIndicatorController.assignBox(value)
        updateSessionChrome()
      }}
      onFooterSummaryBoxRef={(value) => {
        sessionChromeRenderController.assignFooterSummaryBox(value)
        updateSessionChrome()
      }}
      onHotkeysOverlayBoxRef={(value) => {
        assignDialogOverlayBox(value)
        renderHotkeysOverlay()
      }}
    />
  )
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
