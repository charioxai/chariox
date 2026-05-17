import process from "node:process"
import { randomBytes } from "node:crypto"
import { homedir } from "node:os"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, MouseButton, ScrollBoxRenderable, TextAttributes, TextRenderable, addDefaultParsers, parseKeypress, type Renderable, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js"
import { createStore, reconcile } from "solid-js/store"

import type {
  AgentInstance,
  BootstrapState,
  PromptQueueItem,
  RuntimeAttachment,
  RuntimeInteraction,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryCursor,
  SessionHistoryEntry,
  SliceRecord,
  TerminalOutputRecord,
  TranscriptEntry,
  WorkflowDefinition,
} from "./cli-types.js"
import {
  createCommandActionHandlers,
} from "./command-actions.js"
import {
  startCliAutomationServer,
  stopCliAutomationServer,
} from "./cli-automation.js"
import { createAgentInteractionStripController } from "./agent-interaction-strip-controller.js"
import { createAttachedSessionPrimeController } from "./attached-session-prime-controller.js"
import { createAssistantMessageCompletionController } from "./assistant-message-completion-controller.js"
import { createAuthoritativeIdleController } from "./authoritative-idle-controller.js"
import { createCliAutomationActionHandler } from "./cli-automation-handler.js"
import { createCliAutomationServerController } from "./cli-automation-server-controller.js"
import { createCliAutomationSnapshotController } from "./cli-automation-snapshot-controller.js"
import { createCliClosingStateController } from "./cli-closing-state-controller.js"
import {
  ATTACHED_PROMPT_PLACEHOLDER,
  CHROME_UPDATE_THROTTLE_MS,
  COMMAND_CENTER_OVERLAY_FOOTPRINT,
  LIVE_TRANSCRIPT_LIMIT,
  LIVE_TRANSCRIPT_MAX_CHARS,
  PROMPT_KEYBINDINGS,
  STREAM_BATCH_WINDOW_MS,
  TURN_COMPLETION_QUIET_MS,
} from "./cli-runtime-tuning.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import { createDetachedKernelConnectController } from "./detached-kernel-connect-controller.js"
import { createAgentFocusTransitionController } from "./agent-focus-transition-controller.js"
import { formatAgentLabel, formatAgentLocationLabel } from "./agent-label.js"
import { createAgentRuntimeProjectionController } from "./agent-runtime-projection-controller.js"
import {
  describeCliDialogFocusTarget,
  type CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"
import { createCliDialogOverlayController } from "./cli-dialog-overlay-controller.js"
import {
  renderCliDialogOverlay,
} from "./cli-dialog-overlay.js"
import { createCliLoadingStateController } from "./cli-loading-state-controller.js"
import { createCliPollingController } from "./cli-polling-controller.js"
import { createCliProcessLifecycleController } from "./cli-process-lifecycle-controller.js"
import { createCliStdinKeyController } from "./cli-stdin-key-controller.js"
import { createBackgroundPollerStartupController } from "./background-poller-startup-controller.js"
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
import { createProviderNamespaceSubmitController } from "./provider-namespace-submit-controller.js"
import { createProviderPromptProjectionController } from "./provider-prompt-projection-controller.js"
import { createClipboardController } from "./clipboard-controller.js"
import {
  createFooterFlashController,
  type FooterFlash,
} from "./footer-flash-controller.js"
import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"
import { createHotkeyDebugReporter } from "./hotkey-debug-reporter.js"
import { createHotkeysToggleController } from "./hotkeys-toggle-controller.js"
import { createHistoryLoadingRenderController } from "./history-loading-render-controller.js"
import { createHistoryScrollRestoreController } from "./history-scroll-restore-controller.js"
import { clampScrollTop } from "./history-viewport.js"
import { renderHistoryLoadingIndicator as renderHistoryLoadingIndicatorView } from "./history-loading-renderer.js"
import { createDefaultShellContext, type ShellContext } from "@arroba/kernel-client/shell-core"
import { createFocusedInteractionChoiceController } from "./focused-interaction-choice-controller.js"
import { createGlobalKeyboardShortcutController } from "./global-keyboard-shortcut-controller.js"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"
import { createKernelEventDispatchController } from "./kernel-event-dispatch-controller.js"
import { createKernelEventController } from "./kernel-event-controller.js"
import { runClaudeNativeTui } from "./native-tui/claude.js"
import { runCodexNativeTui } from "./native-tui/codex.js"
import { runOpenCodeNativeTui } from "./native-tui/opencode.js"
import {
  getUserConfig,
  getUserConfigSchema,
  setUserConfigValue,
  unsetUserConfigValue,
} from "./config-api.js"
import {
  acceptCloudSessionInvite,
  createCloudSessionInvite,
  createSessionInvite,
  joinSessionInvite,
  listCloudCollaborators,
  listCloudSessionMembers,
} from "./cloud-session-api.js"
import { createSessionBrowserController } from "./session-browser-controller.js"
import { createSlashCommandSubmitController } from "./slash-command-submit-controller.js"
import {
  clampSessionBrowserIndex,
} from "./session-browser-key-policy.js"
import { createSessionBrowserProjectionController } from "./session-browser-projection-controller.js"
import {
  aliasAgent,
  cycleAgentFocus as cycleAgentFocusApi,
  destroyAgent as destroyAgentApi,
  focusAgent as focusAgentApi,
  spawnAgent as spawnAgentApi,
  updateAgentConfig,
  updateAgentProfile,
  updateAgentSubstitutes,
} from "./agent-api.js"
import {
  getMcpServer,
  getSkill,
  grantAgentMcp,
  grantAgentSkill,
  importMcpServers,
  importSkills,
  installMcpServer,
  installSkill,
  listMcpServers,
  listSkills,
  revokeAgentMcp,
  revokeAgentSkill,
  uninstallMcpServer,
  uninstallSkill,
  updateMcpServer,
  updateSkill,
} from "./extension-api.js"
import { deleteKernel } from "./kernel-api.js"
import { createKernelEventSubscriptionController } from "./kernel-event-subscription-controller.js"
import { createKernelRestartRecoveryController } from "./kernel-restart-recovery-controller.js"
import { createKernelResyncController } from "./kernel-resync-controller.js"
import { createKernelSessionSnapshotController } from "./kernel-session-snapshot-controller.js"
import { createKernelSessionUnavailableController } from "./kernel-session-unavailable-controller.js"
import {
  createCliProcessLoggerRegistry,
  formatCliError,
} from "./cli-process-logging.js"
import { runLogViewer } from "./logs.js"
import {
  createConnectionHealthWatchdogController,
} from "./connection-health-watchdog-controller.js"
import { createDaemonActivityController } from "./daemon-activity-controller.js"
import {
  bootstrapCliRuntime,
} from "./cli-runtime-bootstrap.js"
import {
  createCliRuntimeDebugLogger,
} from "./cli-runtime-debug-logger.js"
import {
  createCliUiBatchController,
} from "./cli-ui-batch-controller.js"
import {
  bootstrapCloudRelayProfile,
} from "./cloud-relay.js"
import { createCliExitController } from "./cli-exit-controller.js"
import {
  resolveConfiguredCloudRelayApiUrl,
} from "./cli-options.js"
import { openExternalUrl } from "./external-url.js"
import {
  mergeRelayCloudProfile,
  mergeUiPreferences,
  relayCloudProfile,
  resolveMaxAgentsPerScreen,
  saveProviderPreferences,
  saveRelayCloudProfile,
  saveSessionPromptState,
  sessionPromptDraftEntry,
  saveUiPreferences,
  sessionPromptHistoryEntries,
  type ArrobaPreferences,
  type MultiAgentResponseLayout,
} from "./preferences.js"
import { createPromptAttachmentController } from "./prompt-attachment-controller.js"
import {
  createPromptAttachmentHighlightController,
} from "./prompt-attachment-highlight-controller.js"
import { createPromptAttachmentIntakeController } from "./prompt-attachment-intake-controller.js"
import {
  type PendingPromptAttachment,
} from "./prompt-attachment-state.js"
import {
  createPromptDraftPersistController,
} from "./prompt-draft-persist-controller.js"
import { createPromptFocusRetentionController } from "./prompt-focus-retention-controller.js"
import { createPromptHistoryAttachmentController } from "./prompt-history-attachment-controller.js"
import { createPromptSurfaceMouseController } from "./prompt-surface-mouse-controller.js"
import {
  createPromptHistoryNavigationController,
} from "./prompt-history-navigation-controller.js"
import { createPromptHistoryRestoreController } from "./prompt-history-restore-controller.js"
import { createPromptKeyDownController } from "./prompt-keydown-controller.js"
import { createPromptChromeProjectionController } from "./prompt-chrome-projection-controller.js"
import { createPromptSessionHistoryController } from "./prompt-session-history-controller.js"
import { createPromptSubmissionAgentStateController } from "./prompt-submission-agent-state-controller.js"
import {
  createPromptPlaceholderSyncController,
  derivePromptInputMaxHeight,
} from "./prompt-surface-state.js"
import {
  createPromptInputHistoryRefreshController,
} from "./prompt-input-history-refresh-controller.js"
import { createPromptInputHistoryController } from "./prompt-input-history-controller.js"
import {
  createPromptSubmissionUiController,
} from "./prompt-submission-ui-controller.js"
import { createPromptSubmitCoordinator } from "./prompt-submit-coordinator.js"
import { createNormalPromptSubmitController } from "./normal-prompt-submit-controller.js"
import { createPollerDegradationController } from "./poller-degradation-controller.js"
import {
  createPromptTextController,
} from "./prompt-text-controller.js"
import { createPromptTurnNavigationController } from "./prompt-turn-navigation-controller.js"
import { createPromptStopController } from "./prompt-stop-controller.js"
import { createPrimaryTranscriptEntryController } from "./primary-transcript-entry-controller.js"
import { createPrimaryTranscriptRenderController } from "./primary-transcript-render-controller.js"
import { createPrimaryTranscriptRuntimeStoreController } from "./primary-transcript-runtime-store-controller.js"
import {
  createTurnCompletionController,
} from "./turn-completion-controller.js"
import {
  preparePromptAttachmentsForSubmit,
  promptAttachmentTransferIsForced,
} from "./prompt-attachment-transfer.js"
import {
  promptAttachmentTokenKind,
  promptAttachmentTokenStyle,
  promptAttachmentTokenStyleIds,
} from "./prompt-attachment-tokens.js"
import {
  cancelActivePrompt,
  respondToInteraction,
  submitPromptWithRecovery,
} from "./prompt-runtime-api.js"
import {
  getPromptInputHistory,
  getSessionHistory,
  recordPromptInputHistory,
} from "./session-history-api.js"
import {
  createPromptMetaRenderController,
  type PromptMetaRenderableRefKey,
} from "./prompt-meta-render-controller.js"
import { renderPromptMeta } from "./prompt-meta-renderer.js"
import {
  type BackendProviderId,
  normalizeBackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  type ProviderCommandCatalogs,
} from "./provider-command-catalog.js"
import { createProviderActivityController } from "./provider-activity-controller.js"
import {
  getProviderAuthStatus,
  getProviderCatalog,
  getProviderCommandCatalogs,
  getProviderRun,
  launchProviderRun,
  listProviderProcesses,
  logoutProvider,
  sameProviderRun,
  startProviderLogin,
  teardownProviderProcesses,
  tryGetProviderRun,
  updateSessionConfig,
} from "./provider-api.js"
import { createProviderSelectionController } from "./provider-selection-controller.js"
import { createProviderRecoveryController } from "./provider-recovery-controller.js"
import {
  createPromptContentChangeController,
} from "./prompt-content-change-controller.js"
import { createPromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { createPromptInputRefController } from "./prompt-input-ref-controller.js"
import { createPromptSessionStatePersistenceController } from "./prompt-session-state-persistence-controller.js"
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
  getPollRecoveryDecision,
  getProviderActivityLabel,
  getSessionStatusLabel,
  getTurnCompletionDelayMs,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
import {
  applyProviderRunProfileToSession,
  deriveFocusedStatusBadge,
} from "./session-chrome-state.js"
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
  activeInteractionForAgent as activeInteractionForAgentForSession,
  agentHasPromptWork,
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  buildDetachedSessionState,
  focusedAgentIdForSession,
  sessionHasPromptWork,
  sessionResponseLayout,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "./session-state.js"
import { createSessionStateApplyController } from "./session-state-apply-controller.js"
import { createSessionAttachmentController } from "./session-attachment-controller.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createTranscriptHistoryLoadController } from "./transcript-history-load-controller.js"
import {
  trimSingleTrailingNewline,
} from "./transcript-text.js"
import { resolveTerminalRecordAgentId as resolveTerminalRecordAgentIdFromState } from "./terminal-record-agent-resolver.js"
import { createTranscriptHistoryAutoloadController } from "./transcript-history-autoload-controller.js"
import { createTranscriptScrollMonitorController } from "./transcript-scroll-monitor-controller.js"
import { createTranscriptScrollboxRefController } from "./transcript-scrollbox-ref-controller.js"
import {
  createTerminalOutputRecordQueue,
} from "./terminal-output-record-queue.js"
import { createTerminalOutputRecordProcessor } from "./terminal-output-record-processor.js"
import { createTerminalExitController } from "./terminal-exit-controller.js"
import { createTerminalResizeController } from "./terminal-resize-controller.js"
import { createTranscriptViewportController } from "./transcript-viewport-controller.js"
import { createTranscriptRenderDeferralController } from "./transcript-render-deferral-controller.js"
import { createTranscriptParserRegistration } from "./transcript-parser-registration.js"
import { createVisibleActivityLabelController } from "./visible-activity-label-controller.js"
import { createWorkingAnimationController } from "./working-animation-controller.js"
import {
  shouldRenderProviderStatus,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import {
  decideBootstrapAction,
  SESSION_NEW_PLACEHOLDER,
  formatSessionList,
  selectAttachableSession,
  type SessionListEntry,
} from "./sessions.js"
import {
  aliasSession,
  archiveSessionById,
  attachToSession,
  createSession,
  deleteSessionByRef,
  detachSessionAttachment,
  getSessionState,
  listSessions,
  resolveSession,
} from "./session-api.js"
import { isSessionUnavailableError } from "./session-errors.js"
import {
  catchUpAttachedSession,
  pollRuntimeNotices,
  pumpTerminalOutput,
  resizeSessionTerminal as maybeResize,
} from "./session-runtime-api.js"
import {
  attachWorkspaceLink,
  createWorkspaceLink,
  detachWorkspaceLink,
  listWorkspaceLinks,
  showWorkspaceLink,
} from "./workspace-link-api.js"
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
import { applyTheme, createTranscriptSyntaxStyle, setThemeRegistry, theme } from "./theme.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomActivationController } from "./waiting-room-activation-controller.js"
import { createWaitingRoomReconcileController } from "./waiting-room-reconcile-controller.js"
import {
  getWaitingRoomInventory,
  type RemoteKernelView,
  type RemoteMachineView,
} from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import { createWaitingRoomIntroAnimationController } from "./waiting-room-intro-animation-controller.js"
import { createWaitingRoomRefreshIntervalController } from "./waiting-room-refresh-interval-controller.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomFocus, WaitingRoomState } from "./waiting-room-types.js"
import { createWaitingRoomTransitionController } from "./waiting-room-transition-controller.js"
import { createWaitingRoomHiddenKernelController } from "./waiting-room-hidden-kernel-controller.js"
import { createWaitingRoomLifecycleActionController } from "./waiting-room-lifecycle-action-controller.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import { createWaitingRoomKeyController } from "./waiting-room-key-controller.js"
import {
  type WorkspaceScreenMode,
} from "./workspace-screen.js"
import {
  type WorkspaceShellEntry,
} from "./workspace-shell.js"
import {
  deriveWorkspaceShellContextForSession,
  submitWorkspaceShellCommand as submitWorkspaceShellCommandWithDeps,
} from "./workspace-shell-controller.js"
import {
  createWorkflowController,
  createWorkflowSelectionSyncController,
} from "./workflow-controller.js"
import {
  buildWorkflowInspectorProjection,
  type WorkflowInspectorMode,
} from "./workflow-inspector-projection.js"
import {
  deriveWorkflowPromptState,
} from "./workflow-prompt-state.js"
import {
  createWorkflowNodeInstructionsEditorController,
  type WorkflowNodeInstructionsEditor,
} from "./workflow-node-instructions-editor-controller.js"
import { createWorkflowPromptSubmitController } from "./workflow-prompt-submit-controller.js"
import { createWorkflowTerminalPanelController } from "./workflow-terminal-panel-controller.js"
import { WorkspaceLayout } from "./workspace-layout.js"
import {
  approveRemoteMachine,
  forgetRemoteMachine,
  listRemoteMachineKernels,
  listRemoteMachines,
  renameRemoteMachine,
} from "./remote-machine-api.js"
import {
  configureRelay,
  connectKernelCloudRelay,
  createTerminalPairingLink,
  getRelayStatus,
  issueKernelCloudRelayClientToken,
  logoutCloudRelay,
  pairKernelCloudRelayClient,
  pairKernelCloudRelayMachine,
  pollCloudRelayLogin,
  renderTerminalPairingQr,
  startCloudRelayLogin,
  type RelayStatusView,
  type TerminalPairingLinkView,
  type TerminalTypeView,
  type TerminalView,
} from "./relay-api.js"
import {
  createSlice,
  deleteSlice,
  getSlice,
  getSliceDisplayEndpoint,
  importSliceProviderAuth,
  listSlices,
  startSlice,
  stopSlice,
} from "./slice-api.js"
import {
  computeCurrentTurnId,
  computeNextTurnId,
  formatTranscriptPreview,
  previewLineForTerminalRecord,
} from "./transcript-preview.js"
import {
  buildTranscriptEntryRenderable,
  renderPromptTranscript,
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
  const { client, options } = props.bootstrap
  const supportsKernelEventStream = client.supportsKernelEvents()
  const launchedDetached = Boolean(options.detached && !props.bootstrap.binding)
  const initialBinding = props.bootstrap.binding
  const initialSession = initialBinding?.session ?? buildDetachedSessionState(options)
  const appLogger = getLogger("cli.app", {
    session_id: initialBinding?.session.id ?? null,
    attachment_id: initialBinding?.attachment.id ?? null,
    client_id: options.clientId,
  })
  const renderer = useRenderer()
  const dimensions = useTerminalDimensions()
  const initialEntries = initialBinding?.historyEntries ?? []
  const initialSessions = props.bootstrap.sessions
  const initialProviderCatalog = props.bootstrap.providerCatalog
  const initialProviderCommandCatalogs = props.bootstrap.providerCommandCatalogs
  const initialPreferences = props.bootstrap.preferences
  const initialThemeRegistry = props.bootstrap.themeRegistry ?? DEFAULT_THEME_REGISTRY
  setThemeRegistry(initialThemeRegistry)
  const initialThemeId = applyTheme(initialPreferences.ui?.theme, initialThemeRegistry)
  const initialPromptHistory = initialBinding?.promptHistoryEntries
    ?? (initialBinding?.session
      ? sessionPromptHistoryEntries(initialPreferences, initialBinding.session.id)
      : [])
  const initialPromptDraft = initialBinding?.session
    ? sessionPromptDraftEntry(initialPreferences, initialBinding.session.id)
    : ""
  const [preferencesState, setPreferencesState] = createSignal<ArrobaPreferences>(initialPreferences)
  const [themeRevision, setThemeRevision] = createSignal(0)
  const maxAgentsPerScreen = () => resolveMaxAgentsPerScreen(preferencesState().ui?.maxAgentsPerScreen)
  const [sessionState, setSessionState] = createSignal(initialSession)
  const [attachmentState, setAttachmentState] = createSignal<RuntimeAttachment | null>(initialBinding?.attachment ?? null)
  const [providerRunState, setProviderRunState] = createSignal<RuntimeProviderRun | null>(initialBinding?.providerRun ?? null)
  const [createdSessionState, setCreatedSessionState] = createSignal(initialBinding?.createdSession ?? false)
  const [availableSessions, setAvailableSessions] = createSignal<SessionListEntry[]>(initialSessions)
  const [providerCatalogState, setProviderCatalogState] = createSignal<ProviderCatalog>(initialProviderCatalog)
  const [providerCommandCatalogState, setProviderCommandCatalogState] = createSignal<ProviderCommandCatalogs>(initialProviderCommandCatalogs)
  const [themeRegistryState] = createSignal(initialThemeRegistry)
  const [relayStatusState, setRelayStatusState] = createSignal<RelayStatusView | null>(null)
  const [remoteMachinesState, setRemoteMachinesState] = createSignal<RemoteMachineView[]>([])
  const [remoteKernelsState, setRemoteKernelsState] = createSignal<RemoteKernelView[]>([])
  const [slicesState, setSlicesState] = createSignal<SliceRecord[]>([])
  const [terminalsState, setTerminalsState] = createSignal<TerminalView[]>([])
  const [waitingRoomInventoryStatus, setWaitingRoomInventoryStatus] = createSignal<"loading" | "ready" | "error">("loading")
  const waitingRoomHiddenKernelController = createWaitingRoomHiddenKernelController({
    initialHiddenKernelIds: initialPreferences.ui?.hiddenRemoteKernelIds ?? [],
    persistHiddenKernelIds: (hiddenKernelIds) => {
      void saveUiPreferences({ hiddenRemoteKernelIds: hiddenKernelIds })
      setPreferencesState((current) => mergeUiPreferences(current, { hiddenRemoteKernelIds: hiddenKernelIds }))
    },
  })
  const [waitingRoomCloudNotice, setWaitingRoomCloudNotice] = createSignal<string | null>(null)
  const [terminalPairingOpen, setTerminalPairingOpen] = createSignal(false)
  const [terminalPairingState, setTerminalPairingState] = createSignal<TerminalPairingLinkView | null>(null)
  const [terminalPairingQrLines, setTerminalPairingQrLines] = createSignal<string[]>([])
  const [sessionBrowserOpen, setSessionBrowserOpen] = createSignal(false)

  const agentLocationLabel = (agent: AgentInstance | null | undefined): string | null =>
    formatAgentLocationLabel(agent, slicesState())
  const [sessionBrowserIndex, setSessionBrowserIndex] = createSignal(0)
  const [waitingRoomState, setWaitingRoomState] = createSignal<WaitingRoomState>(
    createWaitingRoomState(
      initialSessions,
      initialProviderCatalog,
      (options.provider ?? "opencode") as BackendProviderId,
      options.model,
      options.effort,
      initialThemeId,
      initialThemeRegistry,
    ),
  )
  const initialWorkspaceTarget = initialSession.workspace_id || options.workspace || process.cwd()
  const initialWorktreeTarget = initialSession.worktree_id || options.worktree || initialWorkspaceTarget
  const [pendingWorkspaceTarget, setPendingWorkspaceTarget] = createSignal(initialWorkspaceTarget)
  const [pendingWorktreeTarget, setPendingWorktreeTarget] = createSignal(initialWorktreeTarget)
  const [multiAgentResponseLayout, setMultiAgentResponseLayout] = createSignal<MultiAgentResponseLayout>(
    sessionResponseLayout(initialSession, preferencesState().ui?.multiAgentResponseLayout),
  )
  const [entries, setEntries] = createStore<TranscriptEntry[]>(initialEntries)
  const [activeStatusLabel, setActiveStatusLabel] = createSignal<string | null>(null)
  const [providerActivityLabel, setProviderActivityLabel] = createSignal<string | null>(null)
  const [agentActivityLabels, setAgentActivityLabels] = createSignal<Record<string, string | null>>({})
  const [streamingAgentId, setStreamingAgentId] = createSignal<string | null>(initialSession.active_prompt?.target_agent_id ?? null)
  const [statusLine, setStatusLine] = createSignal(DEFAULT_CONNECTED_STATUS)
  const [fatalError, setFatalError] = createSignal<string | null>(null)
  const [submitting, setSubmitting] = createSignal(false)
  const [entryCounter, setEntryCounter] = createSignal(initialEntries.length)
  const [daemonDisconnected, setDaemonDisconnected] = createSignal(false)
  const [kernelConnected, setKernelConnected] = createSignal(!launchedDetached)
  const [nextHistoryCursor, setNextHistoryCursor] = createSignal<SessionHistoryCursor | null>(null)
  const [agentPanePreviews, setAgentPanePreviews] = createSignal<Record<string, string>>({})
  const [agentPaneEntries, setAgentPaneEntries] = createSignal<Record<string, TranscriptEntry[]>>({})
  const [agentBusyLatches, setAgentBusyLatches] = createSignal<Record<string, boolean>>({})
  const [sessionHydrating, setSessionHydrating] = createSignal(false)
  const [loadingHistory, setLoadingHistory] = createSignal(false)
  const [workingAnimationFrame, setWorkingAnimationFrame] = createSignal(0)
  const [working, setWorking] = createSignal(sessionHasPromptWork(initialSession))
  const [footerFlash, setFooterFlash] = createSignal<FooterFlash | null>(null)
  const [pendingAttachments, setPendingAttachments] = createSignal<PendingPromptAttachment[]>([])
  const [promptHistoryEntries, setPromptHistoryEntries] = createSignal<string[]>(initialPromptHistory)
  const [promptHistoryIndex, setPromptHistoryIndex] = createSignal<number | null>(null)
  const [promptHistoryDraft, setPromptHistoryDraft] = createSignal<string | null>(null)
  const [hotkeysOpen, setHotkeysOpen] = createSignal(false)
  const [expandedTurnIdsByAgent, setExpandedTurnIdsByAgent] = createSignal<Record<string, number[]>>({})
  const [workspaceScreenMode, setWorkspaceScreenMode] = createSignal<WorkspaceScreenMode>("agents")
  const [workspaceShellContext, setWorkspaceShellContext] = createSignal<ShellContext>(createDefaultShellContext({
    workspace: initialWorkspaceTarget,
    worktree: initialWorktreeTarget,
    sessionId: initialBinding ? initialSession.id : undefined,
    agentId: initialBinding ? initialSession.focused_agent_id ?? initialSession.agents[0]?.id : undefined,
    provider: options.provider ?? "opencode",
    model: options.model ?? "default",
    effort: options.effort || "medium",
  }))
  const [workspaceShellEntries, setWorkspaceShellEntries] = createSignal<WorkspaceShellEntry[]>([])
  const [workspaceShellEntryCounter, setWorkspaceShellEntryCounter] = createSignal(0)
  const [selectedWorkflowId, setSelectedWorkflowId] = createSignal<string | null>(initialSession.workflows?.[0]?.id ?? null)
  const [selectedWorkflowNodeId, setSelectedWorkflowNodeId] = createSignal<string | null>(null)
  const [workflowInspectorMode, setWorkflowInspectorMode] = createSignal<WorkflowInspectorMode>("runtime")
  const [workflowNodeInstructionsEditor, setWorkflowNodeInstructionsEditor] = createSignal<WorkflowNodeInstructionsEditor | null>(null)
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
  // Connection resilience tracking
  const SILENT_POLL_THRESHOLD = 8 // ~2 seconds of no activity (8 * 250ms polling interval)
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
  const activeInteractionForAgent = (agentId: string | null | undefined): RuntimeInteraction | null =>
    activeInteractionForAgentForSession(sessionState(), agentId)
  const focusedAgentInteraction = () => activeInteractionForAgent(focusedAgentId())
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
  const workflowInspector = () => buildWorkflowInspectorProjection({
    session: sessionState(),
    selectedWorkflowId: selectedWorkflowId(),
    selectedWorkflowNodeId: selectedWorkflowNodeId(),
    inspectorMode: workflowInspectorMode(),
    nodeInstructionsEditor: workflowNodeInstructionsEditor(),
    updateNodeInstructionsDraft: (draft) => workflowNodeInstructionsEditorController.updateDraft(draft),
    setNodeInstructionsInputRef: (editorRef) => {
      workflowNodeInstructionsEditorController.setInputRef(editorRef)
    },
  })
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
  const visibleTranscriptEntries = () => entries.filter((entry) => entry && !entry.hidden)
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
  const focusedStatusBadge = () => deriveFocusedStatusBadge({
    attached: isAttached(),
    daemonDisconnected: daemonDisconnected(),
    activeStatusLabel: focusedActivityLabel(),
    focusedBusy: focusedAgentBusy(),
    agents: allAgentsBusyState(),
  })
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
  const describeRenderableDebug = (renderable: Renderable | null | undefined) => {
    return describeCliDialogFocusTarget(renderable as CliDialogFocusTarget | null | undefined)
  }
  const currentFocusedRenderable = () => (
    (renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null
  )
  const waitingRoomReconcileController = createWaitingRoomReconcileController({
    getCurrentState: waitingRoomState,
    setWaitingRoomState,
    getSessions: availableSessions,
    getProviderCatalog: providerCatalogState,
    getRemoteState: () => ({
      cloudNotice: waitingRoomCloudNotice(),
      inventoryStatus: waitingRoomInventoryStatus(),
      loadingFrame: waitingRoomState().introStep,
      relay: relayStatusState(),
      machines: remoteMachinesState(),
      kernels: remoteKernelsState(),
      terminals: terminalsState(),
      slices: slicesState(),
    }),
    getThemeRegistry: themeRegistryState,
    getCurrentProvider: () => (options.provider ?? "opencode") as BackendProviderId,
    getCurrentModel: () => options.model,
    setProviderDefaults: (defaults) => {
      options.provider = defaults.provider
      options.model = defaults.model
      options.effort = defaults.effort
    },
    applyTheme,
    resetTranscriptSyntax: () => {
      transcriptSyntaxStyleController.reset()
    },
    bumpThemeRevision: () => {
      setThemeRevision((revision) => revision + 1)
    },
    saveUiThemePreference: (themeId) => {
      void saveUiPreferences({ theme: themeId })
    },
    mergeUiThemePreference: (themeId) => {
      setPreferencesState((current) => mergeUiPreferences(current, { theme: themeId }))
    },
    applyResponseLayout: () => applyResponseLayout(),
    renderCommandCenter: () => renderCommandCenter(),
    saveProviderPreferences: (provider, preferences) => {
      void saveProviderPreferences(provider, preferences)
    },
    isAttached,
    rebuildTranscript: () => rebuildTranscript(),
    updateSessionChrome: () => updateSessionChrome(),
    syncCommandCenter: () => syncCommandCenter(),
  })
  const reconcileWaitingRoom = waitingRoomReconcileController.reconcile
  const waitingRoomActivationController = createWaitingRoomActivationController({
    isKernelConnected: kernelConnected,
    connectKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: waitingRoomState,
    getRemoteState: () => ({
      relay: relayStatusState(),
      machines: remoteMachinesState(),
      kernels: remoteKernelsState(),
      terminals: terminalsState(),
      slices: slicesState(),
    }),
    getWorkspaceTarget: pendingWorkspaceTarget,
    getWorktreeTarget: pendingWorktreeTarget,
    getAvailableSessions: availableSessions,
    getProviderCatalog: providerCatalogState,
    getCurrentProvider: () => (options.provider ?? "opencode") as BackendProviderId,
    getCurrentModel: () => options.model,
    getAccountProfile: () => options.accountProfile,
    handleCloudCommand: () => handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] }),
    setPromptText: (text) => setPromptText(text),
    focusPrompt: () => {
      promptInputRefController.focus()
    },
    syncCommandCenter: (text) => syncCommandCenter(text),
    openTerminalPairingDialog: () => openTerminalPairingDialog(),
    openSessionBrowserDialog: () => openSessionBrowserDialog(),
    createSession: (workspacePath, worktreePath, launch) => createSession(client, workspacePath, worktreePath, undefined, {
      provider: launch.provider,
      model: launch.model,
      effort: launch.effort,
      account_profile: launch.account_profile,
      execution_mode: launch.execution_mode,
      permission_level: launch.permission_level,
    }, launch.sliceRef),
    attachBinding: (session, createdSession, launch) => attachBinding(session, createdSession, launch),
    flashFooter: (message, tone) => flashFooter(message, tone),
    warn: (message, fields) => appLogger?.warn(message, fields),
    formatError,
  })
  const activateWaitingRoom = waitingRoomActivationController.activate
  const waitingRoomLifecycleConfirmationController = createWaitingRoomLifecycleConfirmationController()
  const waitingRoomInventoryRefreshController = createWaitingRoomInventoryRefreshController({
    isKernelConnected: kernelConnected,
    getInventoryStatus: waitingRoomInventoryStatus,
    setInventoryStatus: setWaitingRoomInventoryStatus,
    getWaitingRoomState: waitingRoomState,
    getInventory: () => getWaitingRoomInventory(client),
    isKernelHidden: waitingRoomHiddenKernelController.isKernelHidden,
    setAvailableSessions,
    setRelayStatus: setRelayStatusState,
    setRemoteMachines: setRemoteMachinesState,
    setRemoteKernels: setRemoteKernelsState,
    setTerminals: setTerminalsState,
    setSlices: setSlicesState,
    reconcileWaitingRoom,
    warn: (message, fields) => appLogger?.warn(message, fields),
    formatError,
  })
  const refreshWaitingRoomDataNow = waitingRoomInventoryRefreshController.refreshNow
  const refreshWaitingRoomData = waitingRoomInventoryRefreshController.refresh
  const detachedKernelConnectController = createDetachedKernelConnectController({
    logInfo: (message, fields) => appLogger?.info(message, fields),
    flashFooter: (message, tone) => flashFooter(message, tone),
    getProviderCatalog: () => getProviderCatalog(client, appLogger),
    getProviderCommandCatalogs: () => getProviderCommandCatalogs(client, appLogger),
    invalidateWaitingRoomInventory: waitingRoomInventoryRefreshController.invalidate,
    setProviderCatalog: setProviderCatalogState,
    setProviderCommandCatalogs: setProviderCommandCatalogState,
    setKernelConnected,
    setDaemonDisconnected,
    refreshWaitingRoomData,
  })
  const connectDetachedKernelFromWaitingRoom = detachedKernelConnectController.connect
  const waitingRoomLifecycleActionController = createWaitingRoomLifecycleActionController({
    isKernelConnected: kernelConnected,
    connectDetachedKernel: () => connectDetachedKernelFromWaitingRoom(),
    getWaitingRoomState: waitingRoomState,
    getRemoteState: () => ({
      cloudNotice: waitingRoomCloudNotice(),
      inventoryStatus: waitingRoomInventoryStatus(),
      loadingFrame: waitingRoomState().introStep,
      relay: relayStatusState(),
      machines: remoteMachinesState(),
      kernels: remoteKernelsState(),
      terminals: terminalsState(),
    }),
    getAvailableSessions: availableSessions,
    setAvailableSessions,
    getProviderCatalog: providerCatalogState,
    getWorkspaceTarget: pendingWorkspaceTarget,
    confirmationController: waitingRoomLifecycleConfirmationController,
    archiveSessionById: (sessionId) => archiveSessionById(client, sessionId),
    deleteSessionByRef: (sessionRef, workspace) => deleteSessionByRef(client, sessionRef, workspace),
    forgetRemoteMachine: (machineRef) => forgetRemoteMachine(client, machineRef),
    getRemoteMachines: remoteMachinesState,
    setRemoteMachines: setRemoteMachinesState,
    getRemoteKernels: remoteKernelsState,
    setRemoteKernels: setRemoteKernelsState,
    hideRemoteKernel: waitingRoomHiddenKernelController.hideKernel,
    invalidateInventory: waitingRoomInventoryRefreshController.invalidate,
    reconcileWaitingRoom,
    refreshWaitingRoomData: () => refreshWaitingRoomData(),
    sessionBrowserOpen,
    closeSessionBrowserDialog: () => closeSessionBrowserDialog(),
    flashFooter: (message, tone) => flashFooter(message, tone),
    warn: (message, fields) => appLogger?.warn(message, fields),
    formatError,
  })
  const applyWaitingRoomSessionLifecycleAction = waitingRoomLifecycleActionController.applyAction
  const providerPromptProjectionController = createProviderPromptProjectionController({
    getProviderRun: focusedProviderRun,
    getFocusedAgent: focusedAgent,
    getWaitingRoomState: waitingRoomState,
    getDefaults: () => ({
      provider: options.provider ?? "opencode",
      model: options.model,
      effort: options.effort,
    }),
    getProviderCatalog: providerCatalogState,
  })
  const currentProviderSelection = providerPromptProjectionController.currentProviderSelection
  const providerSelectionController = createProviderSelectionController({
    currentProviderSelection,
    waitingRoomState,
    availableSessions,
    providerCatalog: providerCatalogState,
    themeRegistry: themeRegistryState,
    preferences: preferencesState,
    defaults: () => ({
      provider: (options.provider ?? "opencode") as BackendProviderId,
      model: options.model,
      effort: options.effort,
    }),
    setDefaults: (selection) => {
      options.provider = selection.provider
      options.model = selection.model
      options.effort = selection.effort
    },
    reconcileWaitingRoom,
    isAttached,
    focusedAgentId,
    providerRunState,
    sessionState,
    updateAgentProfile: (sessionId, agentId, profile) => updateAgentProfile(client, sessionId, agentId, profile),
    applySessionState: (session) => applySessionState(session),
    clearProviderRunState: () => setProviderRunState(null),
    getProviderAuthStatus: (provider) => getProviderAuthStatus(client, provider),
    appendNotice: (text) => appendNotice(text),
    flashFooter: (message, tone) => flashFooter(message, tone),
    warn: (message, fields) => appLogger?.warn(message, fields),
    formatError,
  })
  const applyProviderSelection = providerSelectionController.applyProviderSelection
  const applyModelSelection = providerSelectionController.applyModelSelection
  const applyVariantSelection = providerSelectionController.applyVariantSelection
  const promptMetaParts = providerPromptProjectionController.promptMetaParts
  const promptUsageMeta = providerPromptProjectionController.promptUsageMeta
  const currentModelId = providerPromptProjectionController.currentModelId
  const currentVariantId = providerPromptProjectionController.currentVariantId
  const waitingRoomTargets = () => ({
    workspacePath: pendingWorkspaceTarget(),
    worktreePath: pendingWorktreeTarget(),
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
  const promptChromeProjectionController = createPromptChromeProjectionController({
    daemonDisconnected,
    working,
    hasActivePrompt: anyPromptWork,
    submitting,
    queueDepth: focusedQueueDepth,
    fatalError,
    activePromptId: () => focusedActivePrompt()?.id ?? null,
    statusLine,
    isAttached,
    workflowScreenActive: workflowScreenShowing,
    workflowPromptState,
    attachedPlaceholder: ATTACHED_PROMPT_PLACEHOLDER,
    detachedPlaceholder: SESSION_NEW_PLACEHOLDER,
    trackThemeRevision: () => themeRevision(),
    attachedBackground: () => theme.backgroundPanel,
    detachedBackground: () => theme.backgroundElement,
    workflowBackground: () => theme.backgroundElement,
  })
  const sessionStatusMode = promptChromeProjectionController.sessionStatusMode
  const footerHint = promptChromeProjectionController.footerHint
  const promptPlaceholder = promptChromeProjectionController.promptPlaceholder
  const promptAreaBackground = promptChromeProjectionController.promptAreaBackground
  const promptHistoryRestoreController = createPromptHistoryRestoreController({
    getPreferences: () => untrack(preferencesState),
    setPromptHistoryEntries,
    resetPromptHistoryNavigation: () => {
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(null)
    },
    setPromptText: (text) => {
      setPromptText(text)
    },
  })
  const restorePromptHistory = promptHistoryRestoreController.restore
  const promptSessionStatePersistenceController = createPromptSessionStatePersistenceController({
    updatePreferences: (updater) => {
      setPreferencesState((current) => updater(current))
    },
    savePromptState: saveSessionPromptState,
  })
  const persistSessionPromptState = promptSessionStatePersistenceController.persist
  const promptDraftPersistController = createPromptDraftPersistController({
    delayMs: 300,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    persistPromptDraft: ({ sessionId, promptDraft }) =>
      persistSessionPromptState(sessionId, { promptDraft }),
    onPersistError: (error, request) => {
      appLogger?.warn("failed to persist prompt draft", {
        session_id: request.sessionId,
        error: formatError(error),
      })
    },
  })
  const clearPendingPromptDraftPersist = promptDraftPersistController.clearTimer
  const flushPendingPromptDraftPersist = promptDraftPersistController.flush
  const schedulePromptDraftPersist = promptDraftPersistController.schedule
  const clearPromptDraftPersistQueue = promptDraftPersistController.clearPending
  const promptInputHistoryController = createPromptInputHistoryController({
    getCurrentSessionId: () => attachmentState()?.session_id ?? null,
    getAttachmentId: () => attachmentState()?.id ?? null,
    getEntries: promptHistoryEntries,
    setEntries: setPromptHistoryEntries,
    resetNavigation: () => {
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(null)
    },
    clearDraftPersistQueue: clearPromptDraftPersistQueue,
    persistPromptState: persistSessionPromptState,
    recordPromptInputHistory: (sessionId, attachmentId, kind, text) =>
      recordPromptInputHistory(client, sessionId, attachmentId, kind, text),
    onSharedHistoryPersistFailed: (sessionId, error) => {
      appLogger?.warn("failed to persist shared prompt input history", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
    onPromptEchoPersistFailed: (sessionId, error) => {
      appLogger?.warn("failed to persist prompt echo history", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
    onPromptStatePersistFailed: (sessionId, error) => {
      appLogger?.warn("failed to persist session prompt state", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
    onRecordSharedHistoryFailed: (sessionId, error) => {
      appLogger?.warn("failed to record shared prompt input history", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
  })
  const promptHistoryHydrationController = createPromptHistoryHydrationController({
    loadHistory: (sessionId) => getPromptInputHistory(client, sessionId),
    isCurrentSession: (sessionId) => attachmentState()?.session_id === sessionId,
    applyHistory: async (sessionId, nextEntries, latestSequence) => {
      await promptInputHistoryController.replaceFromHydration(sessionId, nextEntries, latestSequence)
    },
  })
  const hydratePromptHistoryFromSession = (sessionId: string): Promise<void> =>
    promptHistoryHydrationController.hydrate(sessionId)
  const appendSharedPromptInputHistory = promptInputHistoryController.appendShared
  const appendPromptEchoToSharedHistory = promptInputHistoryController.appendEcho
  const promptInputHistoryRefreshController = createPromptInputHistoryRefreshController({
    delayMs: 1500,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    refreshHistory: async (sessionId) => {
      const history = await getPromptInputHistory(client, sessionId, promptInputHistoryController.latestSequence(), 500)
      appendSharedPromptInputHistory(sessionId, history.entries)
    },
    onRefreshError: (error, sessionId) => {
      appLogger?.warn("failed to refresh shared prompt input history", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
  })
  const promptSessionHistoryController = createPromptSessionHistoryController({
    currentSessionId: () => attachmentState()?.session_id ?? null,
    navigationDraft: promptHistoryDraft,
    currentPromptText: promptTextController.currentText,
    scheduleHistoryRefresh: promptInputHistoryRefreshController.schedule,
  })
  const scheduleSharedPromptInputHistoryRefresh = promptSessionHistoryController.scheduleSharedRefresh
  const persistablePromptDraft = promptSessionHistoryController.persistableDraft
  const recordPromptAreaHistoryEntry = promptInputHistoryController.recordPromptAreaEntry
  const syncPromptTextSnapshot = promptTextController.syncSnapshot
  const promptAttachmentHighlightController = createPromptAttachmentHighlightController({
    getPromptInput: promptInputRefController.currentOrNull,
    getPendingAttachments: pendingAttachments,
    styleIdForKind: (kind) => promptAttachmentTokenStyleIds[promptAttachmentTokenKind(kind)],
  })
  const refreshPromptAttachmentHighlights = promptAttachmentHighlightController.refresh
  const setPromptText = promptTextController.setText
  const promptInputMaxHeight = () => derivePromptInputMaxHeight({
    attached: isAttached(),
    terminalHeight: dimensions().height,
  })
  const promptFocusRetentionController = createPromptFocusRetentionController({
    delayMs: 0,
    scheduleTimer: startTimeout,
    isAttached,
    focusPromptInput: () => {
      promptInputRefController.focus()
    },
  })
  const retainPromptFocus = promptFocusRetentionController.retainFocus
  const promptHistoryNavigationController = createPromptHistoryNavigationController({
    getPromptText: promptTextController.currentText,
    getEntries: promptHistoryEntries,
    getNavigationIndex: promptHistoryIndex,
    getNavigationDraft: promptHistoryDraft,
    setNavigationIndex: setPromptHistoryIndex,
    setNavigationDraft: setPromptHistoryDraft,
    setPromptText,
    getSessionId: () => attachmentState()?.session_id ?? null,
    schedulePromptDraftPersist,
    retainPromptFocus,
  })
  const navigatePromptHistoryInput = promptHistoryNavigationController.navigate
  const promptPlaceholderSyncController = createPromptPlaceholderSyncController({
    getPromptInput: promptInputRefController.currentOrNull,
    getPlaceholder: promptPlaceholder,
  })
  const syncPromptPlaceholder = promptPlaceholderSyncController.sync
  createEffect(() => {
    promptPlaceholder()
    syncPromptPlaceholder()
  })
  const promptAttachmentController = createPromptAttachmentController({
    getPromptInput: promptInputRefController.currentOrNull,
    getPromptText: promptTextController.currentText,
    setPromptText,
    pendingAttachments,
    setPendingAttachments: (attachments) => setPendingAttachments(attachments),
    updatePendingAttachments: (updater) => setPendingAttachments((current) => updater(current)),
    refreshHighlights: refreshPromptAttachmentHighlights,
    updateSessionChrome: () => updateSessionChrome(),
    requestRender: () => (renderer as { requestRender?: () => void }).requestRender?.(),
  })
  const clearPendingPromptAttachments = promptAttachmentController.clear
  const syncPendingPromptAttachmentsFromText = promptAttachmentController.syncFromText
  const removeLastPendingPromptAttachment = promptAttachmentController.removeLast
  const addPendingPromptAttachments = promptAttachmentController.addStoredFiles
  const removePromptAttachmentsForEdit = promptAttachmentController.removeForEdit

  const promptSubmissionUiController = createPromptSubmissionUiController({
    getSessionId: () => attachmentState()?.session_id ?? null,
    getPendingAttachments: pendingAttachments,
    resetPromptHistoryNavigation: () => {
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(null)
    },
    clearDraftPersistQueue: clearPromptDraftPersistQueue,
    clearPromptText: () => {
      promptTextController.clear()
    },
    setPromptText,
    syncPromptTextSnapshot,
    clearPendingAttachments: clearPendingPromptAttachments,
    setPendingAttachments: (attachments) => setPendingAttachments(attachments),
    refreshAttachmentHighlights: refreshPromptAttachmentHighlights,
    syncCommandCenter,
    retainPromptFocus,
    clearCommandCenter,
    schedulePromptDraftPersist,
    updateSessionChrome: () => updateSessionChrome(),
  })
  const beginSubmittedPromptUi = promptSubmissionUiController.begin
  const restoreFailedPromptUi = promptSubmissionUiController.restore
  const promptHistoryAttachmentController = createPromptHistoryAttachmentController({
    getAttachedSessionId: () => attachmentState()?.session_id ?? null,
    restorePromptHistory,
    invalidateHydration: promptHistoryHydrationController.invalidate,
    hydratePromptHistory: hydratePromptHistoryFromSession,
    isCurrentSession: (sessionId) => attachmentState()?.session_id === sessionId,
    warnHydrationError: (sessionId, error) => {
      appLogger?.warn("failed to hydrate prompt history from session history", {
        session_id: sessionId,
        error: formatError(error),
      })
    },
  })
  createEffect(() => {
    void promptHistoryAttachmentController.sync()
  })
  const sessionBrowserProjectionController = createSessionBrowserProjectionController({
    isAttached,
    availableSessions,
    selectedIndex: sessionBrowserIndex,
    setSelectedIndex: setSessionBrowserIndex,
  })
  const hotkeySections = sessionBrowserProjectionController.hotkeySections
  const sessionBrowserSessions = sessionBrowserProjectionController.sessions
  const normalizeSessionBrowserIndex = sessionBrowserProjectionController.normalizeIndex
  const dialogOverlayController = createCliDialogOverlayController<CliDialogFocusTarget, BoxRenderable>({
    getOpenState: () => ({
      hotkeysOpen: hotkeysOpen(),
      terminalPairingOpen: terminalPairingOpen(),
      sessionBrowserOpen: sessionBrowserOpen(),
    }),
    getCurrentFocus: () => currentFocusedRenderable() as CliDialogFocusTarget | null,
    getPromptFocus: () => promptInputRefController.current() as CliDialogFocusTarget | null | undefined,
    describeFocus: (target) => describeRenderableDebug(target as Renderable | null | undefined),
    scheduleFocusRestore: (callback) => {
      startTimeout(callback, 1)
    },
    setHotkeysOpen,
    setTerminalPairingOpen,
    setSessionBrowserOpen,
    setTerminalPairing: setTerminalPairingState,
    setTerminalPairingQrLines,
    getSessionCount: () => sessionBrowserSessions().length,
    getWaitingRoomSessionIndex: () => waitingRoomState().sessionIndex,
    setSessionBrowserIndex,
    clampSessionBrowserIndex,
    renderOverlay: (mode, onDismiss, overlayBox) => {
      renderCliDialogOverlay({
        overlayBox,
        renderer,
        dimensions: dimensions(),
        mode,
        onDismiss,
        sessions: sessionBrowserSessions(),
        normalizeSessionBrowserIndex,
        terminalPairing: terminalPairingState(),
        terminalPairingQrLines: terminalPairingQrLines(),
        hotkeySections: hotkeySections(),
      })
    },
    createTerminalPairingLink: () => createTerminalPairingLink(client, "cli"),
    renderTerminalPairingQr,
    flashFooter: (message, tone) => flashFooter(message, tone),
    debugHotkey: (message) => hotkeyDebug(message),
    logDebug: (message, fields) => appLogger?.debug(message, fields),
    formatError,
  })
  const dialogOverlayOpen = dialogOverlayController.isOpen
  const closeActiveDialogOverlay = dialogOverlayController.closeActive
  const renderHotkeysOverlay = dialogOverlayController.render
  const closeHotkeys = dialogOverlayController.closeHotkeys
  const closeTerminalPairingDialog = dialogOverlayController.closeTerminalPairing
  const closeSessionBrowserDialog = dialogOverlayController.closeSessionBrowser
  const openHotkeys = dialogOverlayController.openHotkeys
  const openTerminalPairingDialog = dialogOverlayController.openTerminalPairing
  const openSessionBrowserDialog = dialogOverlayController.openSessionBrowser
  const toggleHotkeys = dialogOverlayController.toggleHotkeys
  const promptContentChangeController = createPromptContentChangeController({
    getPromptText: () => promptInputRefController.hasInput() ? promptTextController.currentText() : null,
    isAttached,
    getPreviousSnapshot: promptTextController.snapshot,
    isProgrammaticMutation: promptTextController.isProgrammaticMutation,
    isPromptHistoryActive: () => promptHistoryIndex() !== null || promptHistoryDraft() !== null,
    getSessionId: () => attachmentState()?.session_id,
    getCwd: () => process.cwd(),
    setPromptTextSnapshot: promptTextController.setSnapshot,
    resetPromptHistory: (draft) => {
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(draft)
    },
    syncPendingAttachmentsFromText: syncPendingPromptAttachmentsFromText,
    setPromptText,
    syncCommandCenter,
    schedulePromptDraftPersist,
    attachPromptFiles: (files, insertAt) => attachPromptFiles(files, insertAt),
    onDropFailed: (error, files) => {
      appLogger?.warn("prompt attachment drop failed", {
        error: formatError(error),
        paths: files.map((file) => file.path),
      })
      flashFooter(`failed to attach files: ${formatError(error)}`, "error")
    },
  })
  const handlePromptContentChange = promptContentChangeController.handleChange
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
    entries: () => entries.filter(Boolean),
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

  const sessionBrowserController = createSessionBrowserController({
    isOpen: sessionBrowserOpen,
    visibleSessions: sessionBrowserSessions,
    availableSessions,
    normalizeSelectedIndex: normalizeSessionBrowserIndex,
    setSelectedIndex: (updater) => setSessionBrowserIndex((index) => updater(index)),
    waitingRoomState,
    providerCatalog: providerCatalogState,
    currentProvider: () => (options.provider ?? "opencode") as BackendProviderId,
    currentModel: () => options.model,
    closeDialog: closeSessionBrowserDialog,
    renderOverlay: renderHotkeysOverlay,
    flashFooter,
    attachSession: (session, createNew, launch) => attachBinding(session, createNew, launch),
    applyLifecycleAction: applyWaitingRoomSessionLifecycleAction,
    formatError,
  })
  const handleSessionBrowserKey = sessionBrowserController.handleKey

  const hotkeyDebugReporter = createHotkeyDebugReporter({
    debugLogsEnabled: DEBUG_LOGS_ENABLED,
    logDebug: (message, fields) => appLogger?.debug(message, fields),
    flashFooter,
  })
  const hotkeyDebug = hotkeyDebugReporter.report
  const hotkeysToggleController = createHotkeysToggleController({
    hotkeysOpen,
    toggleHotkeys,
    debugHotkey: hotkeyDebug,
    logDebug: (message, fields) => appLogger?.debug(message, fields),
    currentFocus: currentFocusedRenderable,
    describeFocus: (focus) => describeRenderableDebug(focus as Renderable | null | undefined),
    savedFocusDebug: () => dialogOverlayController.savedFocusDebug(),
  })
  const handleHotkeysToggleShortcut = hotkeysToggleController.handle

  const clipboardController = createClipboardController({
    renderer,
    promptInput: promptInputRefController.currentOrNull,
    flashFooter,
    logWarning: (message, fields) => appLogger?.warn(message, fields),
    formatError,
  })
  const copyPromptSelection = clipboardController.copyPromptSelection
  const copySelection = clipboardController.copySelection
  const promptSurfaceMouseController = createPromptSurfaceMouseController({
    delayMs: 0,
    scheduleTimer: startTimeout,
    isPrimaryButton: (event: { button: MouseButton }) => event.button === MouseButton.LEFT,
    copySelection,
    retainPromptFocus,
  })
  const handlePromptSelectionSurfaceMouseUp = promptSurfaceMouseController.handleMouseUp

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
    entries: () => entries.filter(Boolean),
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

  const terminalOutputRecordProcessor = createTerminalOutputRecordProcessor({
    appendPromptEchoToSharedHistory,
    processKernelTerminalOutputRecord: (record) => {
      kernelEventController.processTerminalOutputRecord(record)
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
    renderMeta: renderPromptMeta,
  })
  const setPromptMetaRenderables = promptMetaRenderController.setRenderables
  const assignPromptMetaRef = (key: PromptMetaRenderableRefKey) => (value: TextRenderable | undefined) => {
    promptMetaRenderController.assignRef(key, value)
    updateSessionChrome()
  }

  const requestTranscriptRender = () => {
    transcriptRenderDeferralController.request()
  }

  const loadingStateController = createCliLoadingStateController({
    getSessionHydrating: sessionHydrating,
    setSessionHydrating,
    setLoadingHistory,
    renderHistoryLoadingIndicator,
    isAttached,
    visibleTranscriptEntryCount: () => visibleTranscriptEntries().length,
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
    getVisibleTranscriptEntries: () => entries.filter(Boolean),
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
    visibleTranscriptEntries: () => entries.filter(Boolean),
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
    const currentEntries = entries.filter(Boolean).map((entry) => ({ ...entry }))
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
    getEntries: () => entries.filter(Boolean),
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
    currentTranscriptEntryCount: () => entries.filter(Boolean).length,
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
  } = createCommandActionHandlers({
    ...(resolveConfiguredCloudRelayApiUrl(preferencesState())
      ? { cloudRelayApiUrl: resolveConfiguredCloudRelayApiUrl(preferencesState()) }
      : {}),
    workspace: initialWorkspaceTarget,
    worktree: initialWorktreeTarget,
    getWorkspaceTarget: pendingWorkspaceTarget,
    getWorktreeTarget: pendingWorktreeTarget,
    setWorkspaceTarget: setPendingWorkspaceTarget,
    setWorktreeTarget: setPendingWorktreeTarget,
    accountProfile: options.accountProfile,
    clientId: options.clientId,
    isAttached,
    sessionState,
    attachmentState,
    providerRunState,
    currentModelId,
    currentVariantId,
    currentProviderId: () => options.provider ?? "opencode",
    focusedAgentId,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    flashFooter,
    appendNotice,
    appendCloudNotice,
    formatError,
    createSession: (workspace, worktree, alias, agentDefaults) => createSession(client, workspace, worktree, alias, agentDefaults),
    createSessionInvite: (sessionId, expiresInMs, maxUses) =>
      createSessionInvite(client, sessionId, expiresInMs, maxUses),
    joinSessionInvite: (inviteToken, userId) => joinSessionInvite(client, inviteToken, userId),
    attachBinding: (session, createdSession) => attachBinding(session, createdSession),
    resolveSession: (reference, workspace) => resolveSession(client, reference, workspace),
    listSessions: () => listSessions(client),
    deleteSessionByRef: (reference, workspace) => deleteSessionByRef(client, reference, workspace),
    deleteKernel: () => deleteKernel(client),
    assignSessionAlias: (sessionId, alias) => aliasSession(client, sessionId, alias),
    aliasAgent: (sessionId, agentId, alias) => aliasAgent(client, sessionId, agentId, alias),
    updateAgentProfile: (sessionId, agentId, options) =>
      updateAgentProfile(client, sessionId, agentId, options),
    transitionToNoSession,
    applyProviderSelection,
    applyModelSelection,
    applyVariantSelection,
    getProviderAuthStatus: (provider) => getProviderAuthStatus(client, provider),
    startProviderLogin: (provider) => startProviderLogin(client, provider),
    logoutProvider: (provider) => logoutProvider(client, provider),
    getRelayStatus: () => getRelayStatus(client),
    configureRelay: (relayUrl, relayToken) => configureRelay(client, relayUrl, relayToken),
    getCloudRelayProfile: () => relayCloudProfile(preferencesState()),
    saveCloudRelayProfile: async (profile) => {
      await saveRelayCloudProfile(profile)
      setPreferencesState((current) => mergeRelayCloudProfile(current, profile))
    },
    bootstrapCloudRelay: (apiUrl, email, accountSlug) =>
      bootstrapCloudRelayProfile({
        apiUrl,
        email,
        ...(accountSlug ? { accountSlug } : {}),
      }),
    startCloudDeviceLogin: (apiUrl, input) => startCloudRelayLogin(client, apiUrl, input),
    pollCloudDeviceLogin: (apiUrl, deviceCode) => pollCloudRelayLogin(client, apiUrl, deviceCode),
    openExternalUrl,
    logoutCloudRelay: (_profile, options) => logoutCloudRelay(client, options),
    pairCloudRelayClient: (_profile, clientId, alias) =>
      pairKernelCloudRelayClient(client, clientId, alias),
    pairCloudRelayMachine: (_profile, machineId, alias) =>
      pairKernelCloudRelayMachine(client, machineId, alias),
    issueCloudKernelRelayToken: async () => connectKernelCloudRelay(client),
    issueCloudMachineRelayToken: async () => connectKernelCloudRelay(client),
    issueCloudClientRelayToken: async (_profile, targetDaemonAlias, tokenOptions) =>
      issueKernelCloudRelayClientToken(
        client,
        targetDaemonAlias,
        options.clientId ?? "arroba-cli",
        tokenOptions?.sessionId ?? null,
      ),
    createCloudSessionInvite: (sessionId, inviteOptions) =>
      createCloudSessionInvite(client, sessionId, inviteOptions),
    acceptCloudSessionInvite: (inviteToken) => acceptCloudSessionInvite(client, inviteToken),
    listCloudSessionMembers: (sessionId) => listCloudSessionMembers(client, sessionId),
    listCloudCollaborators: () => listCloudCollaborators(client),
    getUserConfig: () => getUserConfig(client),
    getUserConfigSchema: () => getUserConfigSchema(client),
    setUserConfigValue: (path, value) => setUserConfigValue(client, path, value),
    unsetUserConfigValue: (path) => unsetUserConfigValue(client, path),
    refreshWaitingRoomData,
    listRemoteMachines: () => listRemoteMachines(client),
    listRemoteMachineKernels: (machineRef) => listRemoteMachineKernels(client, machineRef),
    approveRemoteMachine: (machineRef) => approveRemoteMachine(client, machineRef),
    forgetRemoteMachine: (machineRef) => forgetRemoteMachine(client, machineRef),
    renameRemoteMachine: (machineRef, alias) => renameRemoteMachine(client, machineRef, alias),
    listSlices: async () => {
      const slices = await listSlices(client)
      setSlicesState(slices)
      return slices
    },
    createSlice: async (sliceOptions) => {
      const slice = await createSlice(client, sliceOptions)
      setSlicesState(await listSlices(client))
      return slice
    },
    getSlice: async (sliceRef) => getSlice(client, sliceRef),
    startSlice: async (sliceRef) => {
      const slice = await startSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    stopSlice: async (sliceRef) => {
      const slice = await stopSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    deleteSlice: async (sliceRef) => {
      const slice = await deleteSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    importSliceProviderAuth: async (sliceRef, provider) => importSliceProviderAuth(client, sliceRef, provider),
    getSliceDisplayEndpoint: async (sliceRef) => getSliceDisplayEndpoint(client, sliceRef),
    listProviderProcesses: (provider) => listProviderProcesses(client, provider),
    teardownProviderProcesses: (provider) => teardownProviderProcesses(client, provider),
    listMcpServers: () => listMcpServers(client, pendingWorkspaceTarget()),
    installMcpServer: (config) => installMcpServer(client, pendingWorkspaceTarget(), config),
    updateMcpServer: (config) => updateMcpServer(client, pendingWorkspaceTarget(), config),
    uninstallMcpServer: (name) => uninstallMcpServer(client, pendingWorkspaceTarget(), name),
    importMcpServers: (provider, name) => importMcpServers(client, pendingWorkspaceTarget(), provider, name),
    getMcpServer: (name) => getMcpServer(client, pendingWorkspaceTarget(), name),
    grantAgentMcp: (agentRef, name) => grantAgentMcp(client, pendingWorkspaceTarget(), agentRef, name),
    revokeAgentMcp: (agentRef, name) => revokeAgentMcp(client, agentRef, name),
    listSkills: () => listSkills(client, pendingWorkspaceTarget()),
    installSkill: (sourcePath) => installSkill(client, pendingWorkspaceTarget(), sourcePath),
    updateSkill: (sourcePath) => updateSkill(client, pendingWorkspaceTarget(), sourcePath),
    uninstallSkill: (name) => uninstallSkill(client, pendingWorkspaceTarget(), name),
    importSkills: (provider, name) => importSkills(client, pendingWorkspaceTarget(), provider, name),
    getSkill: (name) => getSkill(client, pendingWorkspaceTarget(), name),
    grantAgentSkill: (agentRef, name) => grantAgentSkill(client, pendingWorkspaceTarget(), agentRef, name),
    revokeAgentSkill: (agentRef, name) => revokeAgentSkill(client, agentRef, name),
    logViewCommand: (fields) => {
      appLogger?.info("handling view command", fields)
      logViewDebug("view command:after set layout", fields)
    },
    setMultiAgentResponseLayout,
    applyResponseLayout,
    updateSessionResponseLayout: (sessionId, attachmentId, layout) =>
      updateSessionConfig(
        client,
        sessionId,
        attachmentId,
        { [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: layout },
        false,
      ),
    updateSessionConfig: (sessionId, attachmentId, values, requiresIdle) =>
      updateSessionConfig(client, sessionId, attachmentId, values, requiresIdle),
    updateAgentConfig: (sessionId, agentId, options) =>
      updateAgentConfig(client, sessionId, agentId, options),
    updateAgentSubstitutes: (sessionId, agentId, action) =>
      updateAgentSubstitutes(client, sessionId, agentId, action),
    applySessionState,
    refreshAgentPanes,
    createWorkspaceLink: (name) => createWorkspaceLink(client, sessionState().id, name),
    listWorkspaceLinks: () => listWorkspaceLinks(client, sessionState().id),
    showWorkspaceLink: (linkRef) => showWorkspaceLink(client, sessionState().id, linkRef),
    attachWorkspaceLink: (linkRef, repoRoot) => attachWorkspaceLink(client, sessionState().id, linkRef, repoRoot),
    detachWorkspaceLink: (linkRef, repoRoot) => detachWorkspaceLink(client, sessionState().id, linkRef, repoRoot),
    openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor,
    getWorkflowNodeInstructionsDraft,
    getWorkflowNodeInstructionsContext,
    openWorkflowTerminalPanel,
    saveUiPreferences: async (prefs) => {
      await saveUiPreferences(prefs)
      setPreferencesState((current) => mergeUiPreferences(current, prefs))
    },
    rebuildTranscript,
    requestRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    },
    afterViewRender: (layout) => {
      startTimeout(() => {
        logViewDebug("view command:post render tick", {
          requested_layout: layout,
          current_focus: describeRenderableDebug(currentFocusedRenderable()),
        })
      }, 0)
    },
    cycleAgentFocus: async () => {
      return trackAgentFocusTransition(async () => {
        const agent = await cycleAgentFocusApi(client, sessionState().id)
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent,
          session,
        }
      })
    },
    launchAgentProviderRun: (provider, model, variant, agentId) =>
      launchProviderRun(
        client,
        sessionState().id,
        provider,
        options.accountProfile,
        model,
        variant,
        agentId,
      ),
    setProviderRunState,
    refreshSessionState: (sessionId) => getSessionState(client, sessionId),
    spawnAgent: async (provider, alias, model, effort, worktreeId, machineRef, worktreePlacement, sliceRef) => {
      const agent = await spawnAgentApi(
        client,
        sessionState().id,
        {
          provider,
          alias,
          model,
          effort,
          worktreeId,
          kernelRef: machineRef,
          worktreePlacement,
          sliceRef,
        },
      )
      return {
        agent,
        session: await getSessionState(client, sessionState().id),
      }
    },
    destroyAgent: async (agentId) => {
      await destroyAgentApi(client, sessionState().id, agentId)
      return getSessionState(client, sessionState().id)
    },
    focusAgent: async (agentId) => {
      return trackAgentFocusTransition(async () => {
        const agent = await focusAgentApi(client, sessionState().id, agentId)
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent,
          session,
        }
      })
    },
    resolveSessionAgent: (reference) => {
      const resolved = resolveSessionAgent(reference)
      return resolved.error
        ? { agent: resolved.agent ?? null, error: resolved.error }
        : { agent: resolved.agent ?? null }
    },
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
    formatAgentLabel,
    refreshSplitPaneFocusRepaint,
    formatSessionList: (sessions, currentSessionId) => formatSessionList(sessions, currentSessionId ?? undefined),
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

  const slashCommandSubmitController = createSlashCommandSubmitController({
    isAttached,
    getSessionId: () => sessionState().id,
    recordPromptAreaHistoryEntry,
    clearPromptText: () => promptTextController.clear(),
    setPromptHistoryIndex,
    setPromptHistoryDraft,
    clearCommandCenter,
    flashFooter,
    logError: (message, fields) => appLogger?.error(message, fields),
    formatError,
    onExit: requestExit,
    onWaiting: requestWaitingRoom,
    onStop: () => requestPromptStop(),
    handleAttachmentCommand: handleAttachmentCommand,
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
  })

  const submitWorkspaceShellCommand = async (rawPrompt: string) => {
    return submitWorkspaceShellCommandWithDeps(rawPrompt, {
      client,
      workspaceShellContext,
      setWorkspaceShellContext: (context) => {
        setWorkspaceShellContext(context)
      },
      nextEntryId: () => {
        const id = workspaceShellEntryCounter() + 1
        setWorkspaceShellEntryCounter((counter) => counter + 1)
        return id
      },
      setWorkspaceShellEntries: (updater) => {
        setWorkspaceShellEntries(updater)
      },
      sessionState,
      refreshSessionState: (sessionId) => getSessionState(client, sessionId),
      applySessionState,
      selectedWorkflowId,
      setSelectedWorkflowId,
      setSelectedWorkflowNodeId,
      rebuildTranscript,
      flashFooter,
      onSessionRefreshError: (sessionId, error) => {
        appLogger?.warn("workspace shell session refresh failed", {
          session_id: sessionId,
          error: formatError(error),
        })
      },
    })
  }

  const workflowPromptSubmitController = createWorkflowPromptSubmitController({
    getWorkflowPromptState: workflowPromptState,
    getPendingAttachmentCount: () => pendingAttachments().length,
    beginSubmittedPromptUi,
    restoreFailedPromptUi,
    invokeWorkflowEndpoint,
    getSessionId: () => sessionState().id,
    recordPromptAreaHistoryEntry,
    flashFooter,
    formatError,
  })

  const providerNamespaceSubmitController = createProviderNamespaceSubmitController({
    getFocusedProvider: focusedBackendProvider,
    workflowScreenShowing,
    getPendingAttachmentCount: () => pendingAttachments().length,
    waitForPendingAgentFocusTransition,
    getFocusedAgentId: focusedAgentId,
    clearActiveToolLabels: primaryTranscriptRuntimeStore.clearActiveToolLabels,
    setProviderActivityLabel,
    setActiveStatusLabel,
    getAttachment: attachmentState,
    getSessionId: () => sessionState().id,
    clearPromptText: () => promptTextController.clear(),
    beginSubmittedPromptUi,
    renderPromptTranscript,
    appendUserPrompt,
    submitProviderNamespacePrompt: (attachmentId, targetAgentId, forwardedPrompt) =>
      submitPromptWithRecovery(
        client,
        sessionState().id,
        attachmentId,
        targetAgentId,
        forwardedPrompt,
        [],
        options,
        appLogger,
      ),
    applySessionState,
    setStreamingAgentId,
    setWorking,
    updateSessionChrome,
    recordPromptAreaHistoryEntry,
    clearCommandCenter,
    restoreFailedPromptUi,
    getSubmittingAgentId: promptSubmissionAgentStateController.getSubmittingAgentId,
    clearAgentBusy,
    setSubmittingAgentId: promptSubmissionAgentStateController.setSubmittingAgentId,
    setSubmitting,
    setFatalError,
    flashFooter,
    logError: (message, fields) => appLogger?.error(message, fields),
    formatError,
  })

  const normalPromptSubmitController = createNormalPromptSubmitController({
    getPendingAttachments: pendingAttachments,
    waitForPendingAgentFocusTransition,
    getFocusedAgentId: focusedAgentId,
    clearActiveToolLabels: primaryTranscriptRuntimeStore.clearActiveToolLabels,
    setProviderActivityLabel,
    setActiveStatusLabel,
    getAttachment: attachmentState,
    getSessionId: () => sessionState().id,
    clearPromptText: () => promptTextController.clear(),
    shouldInlineLocalFiles: () => Boolean(options.relayUrl) || promptAttachmentTransferIsForced(),
    preparePromptAttachmentsForSubmit,
    beginSubmittedPromptUi,
    renderPromptTranscript,
    appendUserPrompt,
    submitPrompt: (attachmentId, targetAgentId, prompt, attachments) =>
      submitPromptWithRecovery(
        client,
        sessionState().id,
        attachmentId,
        targetAgentId,
        prompt,
        attachments,
        options,
        appLogger,
      ),
    applySessionState,
    setStreamingAgentId,
    setWorking,
    updateSessionChrome,
    setStatusLine,
    recordPromptAreaHistoryEntry,
    restoreFailedPromptUi,
    getSubmittingAgentId: promptSubmissionAgentStateController.getSubmittingAgentId,
    clearAgentBusy,
    setSubmittingAgentId: promptSubmissionAgentStateController.setSubmittingAgentId,
    setSubmitting,
    setFatalError,
    flashFooter,
    logInfo: (message, fields) => appLogger?.info(message, fields),
    logError: (message, fields) => appLogger?.error(message, fields),
    formatError,
  })

  const promptSubmitCoordinator = createPromptSubmitCoordinator({
    getPromptText: promptInputRefController.plainText,
    ensureBackgroundPollersStarted: () => ensureBackgroundPollersStarted(),
    getPendingAttachmentCount: () => pendingAttachments().length,
    clearPromptText: () => promptTextController.clear(),
    workflowScreenShowing,
    submitWorkspaceShellCommand: async (rawPrompt) => {
      await submitWorkspaceShellCommand(rawPrompt)
    },
    workflowNodeInstructionsEditorOpen: () => Boolean(workflowNodeInstructionsEditor()),
    submitSlashCommand: async (rawPrompt, submitOptions) =>
      Boolean(await slashCommandSubmitController.submit(rawPrompt, submitOptions)),
    submitProviderNamespacePrompt: (rawPrompt) => providerNamespaceSubmitController.submit(rawPrompt),
    isAttached,
    submitWorkflowPrompt: (rawPrompt) => workflowPromptSubmitController.submit(rawPrompt),
    submitNormalPrompt: (rawPrompt) => normalPromptSubmitController.submit(rawPrompt),
    flashFooter,
    formatError,
  })
  const submitPrompt = promptSubmitCoordinator.submit

  const requestPromptStop = async () => {
    await promptStopController.request()
  }

  const focusedInteractionChoiceController = createFocusedInteractionChoiceController({
    getFocusedInteraction: focusedAgentInteraction,
    isAttached,
    getSessionId: () => sessionState().id,
    getSelectedIndex: interactionChoiceStore.getSelectedIndex,
    setSelectedIndex: interactionChoiceStore.setSelectedIndex,
    getCustomReply: interactionChoiceStore.customReply,
    setCustomReply: interactionChoiceStore.setCustomReply,
    clearCustomReply: interactionChoiceStore.clearCustomReply,
    isCustomEditing: interactionChoiceStore.isCustomEditing,
    setCustomEditing: interactionChoiceStore.setCustomEditing,
    renderAgentInteractions,
    applyResponseLayout,
    respondToInteraction: (sessionId, interactionId, choiceId, customReply) =>
      respondToInteraction(client, sessionId, interactionId, choiceId, customReply),
    applySessionState,
    flashFooter,
    formatError,
  })
  const submitFocusedInteractionChoice = focusedInteractionChoiceController.submitChoice
  const cycleFocusedInteractionChoice = focusedInteractionChoiceController.cycleChoice
  const handleFocusedInteractionKey = focusedInteractionChoiceController.handleKey

  const globalKeyboardShortcutController = createGlobalKeyboardShortcutController({
    handleHotkeysToggleShortcut,
    dialogOverlayOpen,
    closeActiveDialogOverlay,
    requestExit: () => {
      void requestExit()
    },
    requestPromptStop: () => {
      void requestPromptStop()
    },
    activePrompt,
  })
  useKeyboard(globalKeyboardShortcutController.handleKey)
  const handleSigint = globalKeyboardShortcutController.handleSigint
  const promptKeyDownController = createPromptKeyDownController({
    handleFocusedInteractionKey,
    handleCommandCenterKey,
    isAttached,
    promptFocused: promptInputRefController.isFocused,
    commandCenterOpen,
    currentPromptText: () => promptTextController.currentText(),
    promptCursorOffset: () => promptTextController.cursorOffset(),
    promptHistoryIndex,
    promptHistoryDraft,
    navigatePromptHistoryInput,
    handleHotkeysToggleShortcut,
  })
  const handlePromptKeyDown = promptKeyDownController.handleKeyDown
  const promptTurnNavigationController = createPromptTurnNavigationController({
    isAttached,
    getPromptText: () => promptInputRefController.hasInput() ? promptTextController.currentText() : undefined,
    getPromptOffsets: () => visibleTranscriptEntries()
      .filter((entry) => entry.role === "user")
      .map((entry) => primaryTranscriptRuntimeStore.entryWrapperY(entry.id))
      .filter((offset): offset is number => offset !== null),
    getScrollState: transcriptScrollboxRefController.scrollState,
    scrollTo: transcriptScrollboxRefController.scrollTo,
    requestRender: transcriptScrollboxRefController.requestRender,
    setLastTranscriptScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
  })
  const waitingRoomKeyController = createWaitingRoomKeyController({
    isAttached,
    hotkeysOpen: dialogOverlayOpen,
    promptFocused: promptInputRefController.isFocused,
    commandCenterOpen,
    commandCenterQuery: () => commandCenterController.query(),
    getWaitingRoomState: waitingRoomState,
    getSessions: availableSessions,
    getProviderCatalog: providerCatalogState,
    getRemoteState: () => ({
      relay: relayStatusState(),
      machines: remoteMachinesState(),
      kernels: remoteKernelsState(),
      terminals: terminalsState(),
      slices: slicesState(),
    }),
    getThemeRegistry: themeRegistryState,
    reconcileWaitingRoom,
    setWaitingRoomState,
    rebuildTranscript,
    applyLifecycleAction: (action) => {
      void applyWaitingRoomSessionLifecycleAction(action)
    },
    activateWaitingRoom: () => {
      void activateWaitingRoom()
    },
  })
  const stdinKeyController = createCliStdinKeyController({
    parseKeypress: (chunk, options) => parseKeypress(chunk, options),
    dialogOverlayOpen,
    closeActiveDialogOverlay,
    handleSessionBrowserKey,
    requestExit: () => {
      void requestExit()
    },
    handleFocusedInteractionKey,
    promptFocused: promptInputRefController.isFocused,
    commandCenterOpen,
    commandCenterQuery: () => commandCenterController.query(),
    clearCommandCenter,
    toggleWorkspaceScreen,
    isAttached,
    workflowScreenActive,
    cycleWorkflowCanvasNode,
    cycleAgentFocus: () => {
      void handleCycleAgentFocus()
    },
    copyPromptSelection,
    activePrompt,
    requestPromptStop: () => {
      void requestPromptStop()
    },
    removePromptAttachmentsForEdit,
    currentPromptText: () => promptTextController.currentText(),
    pendingAttachmentCount: () => pendingAttachments().length,
    removeLastPendingPromptAttachment,
    handlePromptTurnNavigationKey: (event) => promptTurnNavigationController.handleKey({
      ...event,
      eventType: event.eventType ?? "",
    }),
    handleWaitingRoomKey: waitingRoomKeyController.handleKey,
  })
  const handleStdinData = stdinKeyController.handleData

  const automationSnapshotController = createCliAutomationSnapshotController({
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
  })
  const automationSnapshot = automationSnapshotController.snapshot

  const handleAutomationRequest = createCliAutomationActionHandler({
    client,
    options,
    appLogger,
    snapshot: automationSnapshot,
    isAttached,
    kernelConnected,
    workflowScreenActive,
    setWorkspaceScreenMode,
    rebuildTranscript,
    applyResponseLayout,
    showWorkflowScreen,
    submitWorkspaceShellCommand,
    attachmentState,
    sessionState,
    focusedAgentId,
    setPromptText,
    submitPrompt,
    activateWaitingRoom,
    connectDetachedKernelFromWaitingRoom,
    submitFocusedInteractionChoice,
    cycleFocusedInteractionChoice,
    restoreTerminalAndExit,
    sleep,
  })

  const automationServerController = createCliAutomationServerController({
    socketPath: options.automationSocket,
    handleRequest: handleAutomationRequest,
    startServer: startCliAutomationServer,
    stopServer: stopCliAutomationServer,
    formatError,
    logger: appLogger,
    flashFooter,
  })
  const processLifecycleController = createCliProcessLifecycleController({
    handleSigint,
    handleStdinData,
    startAutomationServer: () => automationServerController.start(),
    stopAutomationServer: () => automationServerController.stop(),
    onSigint: (handler) => process.on("SIGINT", handler),
    offSigint: (handler) => process.off("SIGINT", handler),
    onStdinData: (handler) => process.stdin.on("data", handler),
    offStdinData: (handler) => process.stdin.off("data", handler),
    clearTerminalOutputRecordTimer: () => terminalOutputRecordQueue.clearTimer(),
  })
  processLifecycleController.start()
  onCleanup(processLifecycleController.stop)

  const terminalResizeController = createTerminalResizeController({
    isAttached,
    sessionId: () => sessionState().id,
    resizeSession: (sessionId) => maybeResize(client, sessionId),
  })
  const onResize = terminalResizeController.handleResize

  const pollerDegradationController = createPollerDegradationController({
    connectedStatusLine: DEFAULT_CONNECTED_STATUS,
    logger: appLogger,
    setDaemonDisconnected,
    setStatusLine,
    updateSessionChrome,
    appendNotice,
  })
  const markPollerDegraded = pollerDegradationController.markDegraded
  const markPollerRecovered = pollerDegradationController.markRecovered

  const connectionHealthWatchdogController = createConnectionHealthWatchdogController({
    now: Date.now,
    intervalMs: 250,
    silenceWindowMs: 2000,
    silentThreshold: SILENT_POLL_THRESHOLD,
    scheduleInterval: startInterval,
    clearInterval,
    isClosing: closingStateController.isClosing,
    isAttached,
    isWorking: working,
    onRecover: (decision) => {
      appLogger?.warn("connection appears stale - no activity while working", {
        consecutive_silent_polls: decision.nextConsecutiveSilentPolls,
        time_since_last_activity_ms: decision.timeSinceLastActivityMs,
      })
      if (supportsKernelEventStream) {
        void client.restartKernelEventStream().catch((error) => {
          appLogger?.warn("kernel event stream restart failed", {
            error: formatError(error),
          })
        })
      } else {
        void recoverProviderRun("stale connection - no activity received")
      }
    },
  })

  const daemonActivityController = createDaemonActivityController({
    recordConnectionActivity: () => connectionHealthWatchdogController.recordActivity(),
    daemonDisconnected,
    setDaemonDisconnected,
    updateSessionChrome,
  })
  const recordDaemonActivity = daemonActivityController.record

  const kernelEventController = createKernelEventController({
    recordDaemonActivity,
    recordTurnActivity,
    resolveTerminalRecordAgentId,
    setStreamingAgentId,
    markAgentBusy,
    splitAgentResponseMode,
    visibleTranscriptAgentId,
    focusedAgentId,
    hasTrailingUserPrompt,
    currentAgentPaneEntries,
    computeNextTurnId,
    appendTranscriptEntryToAgentPane,
    appendProviderChunkToAgentPane,
    appendToolUpdateToAgentPane,
    setAgentActivityLabel,
    agentActivityLabel,
    setProviderActivityLabel,
    applyProviderActivity,
    syncVisibleActivityLabel,
    getProviderActivityLabel,
    shouldRenderProviderStatus,
    appendEntry,
    appendProviderChunk,
    appendToolUpdate,
    appendProviderError,
    syncVisibleTranscriptPreview,
    appendAgentPanePreview,
    previewLineForTerminalRecord,
    trimSingleTrailingNewline,
    setDaemonDisconnected,
    setStatusLine,
    updateSessionChrome,
    appendNotice: (message, tone) => appendNotice(message, tone === "warning" ? "warning" : "muted"),
    connectedStatusLine: DEFAULT_CONNECTED_STATUS,
    markAssistantMessageCompleted,
  })

  const kernelSessionSnapshotController = createKernelSessionSnapshotController({
    getSession: sessionState,
    getProviderRun: providerRunState,
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange,
    sessionHasPromptWork,
    applySessionState,
    sameProviderRun,
    logProviderRunDebug,
    setProviderRun: setProviderRunState,
    updateSessionChrome,
    supportsKernelEventStream: () => supportsKernelEventStream,
    recoverProviderRun,
    refreshAgentPanes,
  })
  const applyKernelSessionSnapshot = kernelSessionSnapshotController.apply

  const kernelResyncController = createKernelResyncController({
    getAttachment: attachmentState,
    isAttached,
    getSessionId: () => sessionState().id,
    getSessionStateSnapshot: sessionState,
    catchUpAttachedSession: (sessionId, attachmentId, session) =>
      catchUpAttachedSession(client, sessionId, attachmentId, session, appLogger),
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: providerRunState,
    tryGetProviderRun: (providerRunId) => tryGetProviderRun(client, providerRunId, appLogger),
    sameProviderRun,
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange,
    sessionHasPromptWork,
    applySession: applySessionState,
    applyProviderRun: setProviderRunState,
    refreshAgentPanes,
    clearLocalBusyStateForAuthoritativeIdle,
    onProviderRunCleared: (run, sessionId, reason) => {
      logProviderRunDebug("kernel resync cleared provider run", run, {
        session_id: sessionId,
        reason,
      })
    },
    onProviderRunRefreshed: (run, sessionId, previousProviderRunId, reason) => {
      logProviderRunDebug("kernel resync refreshed provider run", run, {
        session_id: sessionId,
        previous_provider_run_id: previousProviderRunId,
        reason,
      })
    },
    onResyncStart: (sessionId, attachmentId, reason) => {
      appLogger?.info("resyncing attached kernel state", {
        reason,
        session_id: sessionId,
        attachment_id: attachmentId,
      })
    },
    onResyncComplete: (reason) => {
      recordDaemonActivity(`kernel_resync_${reason}`)
      setDaemonDisconnected(false)
      setStatusLine(DEFAULT_CONNECTED_STATUS)
      updateSessionChrome()
    },
    onResyncFailed: (reason, error) => {
      appLogger?.warn("attached kernel resync failed", {
        reason,
        error: formatError(error),
      })
      setDaemonDisconnected(true)
      setStatusLine("Waiting to reconnect to the Arroba kernel.")
      updateSessionChrome()
    },
  })

  const resyncAttachedKernelState = (reason: string) => kernelResyncController.resync(reason)

  const kernelSessionUnavailableController = createKernelSessionUnavailableController({
    isAttached,
    getSession: sessionState,
    getProviderRun: providerRunState,
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    attachToSession: (sessionId) => attachToSession(client, sessionId, options.clientId),
    applyAttachment: setAttachmentState,
    projectSession: applyProviderRunProfileToSession,
    applySession: applySessionState,
    resetKernelEventSubscription: kernelEventSubscriptionController.reset,
    syncKernelEventSubscription,
    refreshAgentPanes,
    clearLocalBusyStateForAuthoritativeIdle,
    recordDaemonActivity,
    onRecovered: () => {
      setDaemonDisconnected(false)
      setStatusLine(DEFAULT_CONNECTED_STATUS)
      updateSessionChrome()
    },
    onStateLookupFailed: (sessionId, message, error) => {
      appLogger?.debug("session unavailable confirmed by state lookup failure", {
        session_id: sessionId,
        message,
        error: formatError(error),
      })
    },
    transitionToNoSession,
  })
  const handleKernelSessionUnavailable = kernelSessionUnavailableController.handle

  const kernelEventDispatchController = createKernelEventDispatchController({
    recordDaemonActivity,
    queueTerminalOutputRecords,
    applyRuntimeNotices: kernelEventController.applyRuntimeNotices,
    applyAssistantMessageCompleted: kernelEventController.applyAssistantMessageCompleted,
    applyKernelSessionSnapshot,
    scheduleSharedPromptInputHistoryRefresh,
    handleKernelSessionUnavailable,
    refreshWaitingRoomData,
    applyTransportResumed: kernelEventController.applyTransportResumed,
    resyncAttachedKernelState,
    appendNotice,
    flashFooter,
    applyTransportClosed: kernelEventController.applyTransportClosed,
    recoverAttachedSessionAfterKernelRestart,
  })
  const handleKernelEvent = kernelEventDispatchController.handleKernelEvent

  const startConnectionWatchdog = connectionHealthWatchdogController.start

  const pollingController = createCliPollingController({
    isClosing: closingStateController.isClosing,
    logger: appLogger,
    formatError,
    isSessionUnavailableError,
    getPollRecoveryDecision,
    onSessionUnavailable: () => {
      transitionToNoSession("Current session is no longer available.")
    },
    onMarkRecovered: markPollerRecovered,
    onMarkDegraded: markPollerDegraded,
    onFatalError: (error) => {
      if (error instanceof Error && /local transport/i.test(error.message)) {
        setDaemonDisconnected(true)
      }
      setFatalError(formatError(error))
      updateSessionChrome()
    },
    sleep,
    isAttached,
    getAttachment: attachmentState,
    getSession: sessionState,
    getProviderRun: providerRunState,
    setProviderRun: setProviderRunState,
    updateSessionChrome,
    recordDaemonActivity,
    queueTerminalOutputRecords,
    pumpTerminalOutput: (sessionId, attachmentId) => pumpTerminalOutput(client, sessionId, attachmentId),
    pollRuntimeNotices: (sessionId, attachmentId) => pollRuntimeNotices(client, sessionId, attachmentId),
    appendNotice: (message) => appendNotice(message),
    sessionHasPromptWork,
    getSessionState: (sessionId) => getSessionState(client, sessionId),
    projectSession: applyProviderRunProfileToSession,
    shouldRefreshAgentPanesForSessionChange,
    applySessionState,
    refreshAgentPanes,
    tryGetProviderRun: (providerRunId) => tryGetProviderRun(client, providerRunId, appLogger),
    sameProviderRun,
    logProviderRunDebug,
    recoverProviderRun,
  })
  const pollOutput = pollingController.pollOutput
  const pollNotices = pollingController.pollNotices
  const pollSessionState = pollingController.pollSessionState

  const backgroundPollerStartupController = createBackgroundPollerStartupController({
    logger: appLogger,
    ready: () => promptInputRefController.hasInput() && transcriptScrollboxRefController.hasScrollbox(),
    promptMounted: promptInputRefController.hasInput,
    transcriptScrollTop: () => transcriptScrollboxRefController.scrollTop(0),
    setLastTranscriptScrollTop: primaryTranscriptRuntimeStore.setLastScrollTop,
    isAttached,
    rebuildTranscript,
    syncPromptPlaceholder,
    focusPrompt: () => {
      promptInputRefController.focus()
    },
    blurPrompt: () => {
      promptInputRefController.blur()
    },
    addResizeListener: () => {
      process.stdout.on("resize", onResize)
    },
    removeResizeListener: () => {
      process.stdout.off("resize", onResize)
    },
    supportsKernelEventStream: () => supportsKernelEventStream,
    syncKernelEventSubscription,
    pollOutput,
    pollNotices,
    pollSessionState,
    startConnectionWatchdog,
    stopConnectionWatchdog: () => {
      connectionHealthWatchdogController.stop()
    },
    logViewDebug,
  })
  const ensureBackgroundPollersStarted = backgroundPollerStartupController.ensureStarted

  onCleanup(() => {
    closingStateController.markClosing()
    backgroundPollerStartupController.stop()
  })

  onCleanup(() => {
    footerFlashController.clearTimer()
    clearPendingPromptDraftPersist()
    cancelPendingTurnCompletion()
    sessionChromeUpdateController.clearTimer()
    promptInputHistoryRefreshController.clearTimer()
  })

  const disposeKernelEventHandler = supportsKernelEventStream
    ? client.onKernelEvent((event) => {
      void handleKernelEvent(event)
    })
    : () => {}

  createEffect(() => {
    void attachmentState()
    void sessionState().id
    void syncKernelEventSubscription()
  })

  onCleanup(() => {
    disposeKernelEventHandler()
    void client.unsubscribeFromKernelEvents().catch(() => {
      // Ignore teardown errors while closing the TUI.
    })
  })

  const transcriptScrollMonitorController = createTranscriptScrollMonitorController({
    intervalMs: 75,
    scheduleInterval: startInterval,
    clearInterval,
    monitorScroll: () => {
      transcriptHistoryAutoloadController.monitorScroll()
    },
  })
  transcriptScrollMonitorController.start()

  onCleanup(() => {
    transcriptScrollMonitorController.stop()
  })

  const workingAnimationController = createWorkingAnimationController({
    intervalMs: 120,
    scheduleInterval: startInterval,
    clearInterval,
    incrementFrame: () => {
      setWorkingAnimationFrame((value) => value + 1)
    },
    sessionStatusMode,
    splitAgentResponseMode,
    updateSessionChrome,
    renderSplitPaneFooters,
  })
  workingAnimationController.start()

  onCleanup(() => {
    workingAnimationController.stop()
  })

  const waitingRoomIntroAnimationController = createWaitingRoomIntroAnimationController({
    intervalMs: 90,
    scheduleInterval: startInterval,
    clearInterval,
    isAttached,
    getWaitingRoomState: waitingRoomState,
    setWaitingRoomState,
    rebuildTranscript,
  })
  waitingRoomIntroAnimationController.start()

  onCleanup(() => {
    waitingRoomIntroAnimationController.stop()
  })

  const waitingRoomRefreshIntervalController = createWaitingRoomRefreshIntervalController({
    intervalMs: 2_500,
    scheduleInterval: startInterval,
    clearInterval,
    refreshWaitingRoomData,
  })
  waitingRoomRefreshIntervalController.start()

  onCleanup(() => {
    waitingRoomRefreshIntervalController.stop()
  })

  onMount(() => {
    if (kernelConnected()) {
      void refreshWaitingRoomData()
      void hydrateCurrentAttachedSession("mount")
    }
  })

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
        dialogOverlayController.assignOverlayBox(value)
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
