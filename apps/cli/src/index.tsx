import process from "node:process"
import { randomBytes } from "node:crypto"
import { homedir } from "node:os"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, MouseButton, ScrollBoxRenderable, TextAttributes, TextRenderable, addDefaultParsers, parseKeypress, type KeyBinding, type Renderable, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js"
import { createStore, reconcile } from "solid-js/store"

import {
  normalizeRuntimeSession,
} from "./cli-types.js"
import type {
  AgentInstance,
  BootstrapState,
  CliOptions,
  RuntimeAttachment,
  RuntimeInteraction,
  RuntimeNoticeRecord,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryCursor,
  SessionHistoryEntry,
  SessionHistoryPageEntry,
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
import { createAttachedSessionPrimeController } from "./attached-session-prime-controller.js"
import { createAssistantMessageCompletionController } from "./assistant-message-completion-controller.js"
import { createAuthoritativeIdleController } from "./authoritative-idle-controller.js"
import { createCliAutomationActionHandler } from "./cli-automation-handler.js"
import { createCliAutomationServerController } from "./cli-automation-server-controller.js"
import { buildCliAutomationSnapshot } from "./cli-automation-snapshot.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  shouldPreserveAgentActivityLabel as shouldPreserveAgentActivityLabelState,
} from "./agent-activity-state.js"
import { createAgentFocusTransitionController } from "./agent-focus-transition-controller.js"
import { formatAgentLabel, formatAgentLocationLabel } from "./agent-label.js"
import {
  describeCliDialogFocusTarget,
  type CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"
import { createCliDialogOverlayController } from "./cli-dialog-overlay-controller.js"
import {
  renderCliDialogOverlay,
} from "./cli-dialog-overlay.js"
import { createCliStdinKeyController } from "./cli-stdin-key-controller.js"
import { createBackgroundPollerStartupController } from "./background-poller-startup-controller.js"
import {
  executeSlashCommand,
} from "./commands.js"
import { createCommandCenterController } from "./command-center-controller.js"
import { renderCommandCenterOverlay } from "./command-center-renderer.js"
import { refreshAgentPaneState, selectCurrentAgentPaneEntries, trimAgentPaneEntries } from "./agent-pane-state.js"
import { createAgentPaneTranscriptEntryController } from "./agent-pane-transcript-entry-controller.js"
import { createAgentPaneTranscriptInteractionController } from "./agent-pane-transcript-interaction-controller.js"
import { createAgentPaneTranscriptRenderController } from "./agent-pane-transcript-render-controller.js"
import { createAgentPaneTranscriptStreamController } from "./agent-pane-transcript-stream-controller.js"
import { createProviderNamespaceSubmitController } from "./provider-namespace-submit-controller.js"
import { createClipboardController } from "./clipboard-controller.js"
import {
  createFooterFlashController,
  type FooterFlash,
} from "./footer-flash-controller.js"
import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"
import { createHotkeysToggleController } from "./hotkeys-toggle-controller.js"
import { buildHotkeySections } from "./hotkey-help.js"
import { createHistoryScrollRestoreController } from "./history-scroll-restore-controller.js"
import { clampScrollTop } from "./history-viewport.js"
import { renderHistoryLoadingIndicator as renderHistoryLoadingIndicatorView } from "./history-loading-renderer.js"
import { createDefaultShellContext, type ShellContext } from "@arroba/kernel-client/shell-core"
import { KernelEvent, LocalIpcClient } from "./ipc.js"
import { createFocusedInteractionChoiceController } from "./focused-interaction-choice-controller.js"
import { createGlobalKeyboardShortcutController } from "./global-keyboard-shortcut-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"
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
  sessionBrowserVisibleSessions,
} from "./session-browser-key-policy.js"
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
import { createProcessLogger, type ArrobaLogger } from "./logging.js"
import { runLogViewer } from "./logs.js"
import { runPollingLoop } from "./polling-effects.js"
import {
  createConnectionHealthWatchdogController,
} from "./connection-health-watchdog-controller.js"
import {
  bootstrapCloudRelayProfile,
} from "./cloud-relay.js"
import { createCliExitController } from "./cli-exit-controller.js"
import {
  applyProviderPreferenceDefaults,
  defaultKernelEndpoint,
  parseArgs,
  resolveConfiguredCloudRelayApiUrl,
} from "./cli-options.js"
import { openExternalUrl } from "./external-url.js"
import {
  inferWorkspaceTargetsFromLaunchDirectory,
} from "./workspace-launch-targets.js"
import {
  isKernelEndpointReachable,
  isKernelEndpointUnavailableError,
  isNoArgDefaultKernelLaunch,
} from "./kernel-endpoint.js"
import {
  loadPreferences,
  mergeSessionPromptState,
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
import {
  createPromptPlaceholderSyncController,
  derivePromptAreaBackground,
  derivePromptInputMaxHeight,
  derivePromptPlaceholder,
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
import type { PromptMetaPart } from "./prompt-meta.js"
import { renderPromptMeta } from "./prompt-meta-renderer.js"
import {
  type BackendProviderId,
  fallbackProviderCatalog,
  isBackendProviderId,
  normalizeBackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  fallbackProviderCommandCatalogs,
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
  hydrateTranscriptEntries,
  stitchPrependedHistory,
} from "./transcript-history.js"
import {
  createPromptContentChangeController,
} from "./prompt-content-change-controller.js"
import { createPromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { buildPaneGridModel } from "./response-pane-grid.js"
import { applyResponsePaneGridLayout } from "./response-pane-grid-layout.js"
import {
  responsePaneRowSlots,
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "./response-panes.js"
import {
  extractPromptHistoryEntries,
} from "./prompt-history.js"
import {
  STATUS_BADGE_WIDTH,
  DEFAULT_CONNECTED_STATUS,
  describeCliError,
  getExitCleanupDecision,
  getPollRecoveryDecision,
  getProviderActivityLabel,
  getSessionStatusLabel,
  getTurnCompletionDelayMs,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
import {
  applyProviderRunProfileToSession,
  deriveAttachedFooterSummary,
  deriveCurrentProviderSelection,
  deriveFooterHint,
  deriveFocusedStatusBadge,
  derivePromptMetaState,
  derivePromptUsageState,
  deriveSessionStatusMode,
  type FocusedStatusBadge,
  type SessionStatusMode,
} from "./session-chrome-state.js"
import {
  createSessionChromeSummaryRenderState,
  renderSessionChromeSummary,
} from "./session-chrome-summary-renderer.js"
import {
  createSessionChromeUpdateController,
} from "./session-chrome-update-controller.js"
import {
  agentHasPromptWork,
  agentPromptState,
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  buildDetachedSessionState,
  sessionHasPromptWork,
  sessionResponseLayout,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "./session-state.js"
import { createSessionStateApplyController } from "./session-state-apply-controller.js"
import { createSessionAttachmentController } from "./session-attachment-controller.js"
import { resolveSessionAgentReference } from "./session-agent-resolver.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createTranscriptHistoryLoadController } from "./transcript-history-load-controller.js"
import {
  applyTranscriptDisplayState,
} from "./transcript-display.js"
import {
  reindexTranscriptEntries,
  trimSingleTrailingNewline,
} from "./transcript-text.js"
import { resolveTerminalRecordAgentId as resolveTerminalRecordAgentIdFromState } from "./terminal-record-agent-resolver.js"
import { createTranscriptHistoryAutoloadController } from "./transcript-history-autoload-controller.js"
import { createTranscriptScrollMonitorController } from "./transcript-scroll-monitor-controller.js"
import {
  createTerminalOutputRecordQueue,
} from "./terminal-output-record-queue.js"
import { createTerminalOutputRecordProcessor } from "./terminal-output-record-processor.js"
import { createTerminalExitController } from "./terminal-exit-controller.js"
import { createTranscriptViewportController } from "./transcript-viewport-controller.js"
import { createTranscriptRenderDeferralController } from "./transcript-render-deferral-controller.js"
import { createWorkingAnimationController } from "./working-animation-controller.js"
import {
  shouldRenderProviderStatus,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import {
  decideBootstrapAction,
  SESSION_NEW_FOOTER_HINT,
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
import {
  createStatusIndicatorRenderState,
  renderStatusIndicator as renderStatusIndicatorView,
} from "./status-indicator-renderer.js"
import { syncAuxiliaryPane } from "./response-layout-render.js"
import { createRenderScheduler } from "./render-scheduler.js"
import {
  createResponsePaneRepaintController,
} from "./response-pane-repaint-controller.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { applyTheme, createTranscriptSyntaxStyle, setThemeRegistry, theme } from "./theme.js"
import { DEFAULT_THEME_REGISTRY, loadThemeRegistry } from "./theme-registry.js"
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
import { createWaitingRoomLifecycleActionController } from "./waiting-room-lifecycle-action-controller.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import { createWaitingRoomKeyController } from "./waiting-room-key-controller.js"
import {
  primeWaitingRoomWorktreeInventory,
} from "./waiting-room-worktrees.js"
import {
  resolveWorkspaceVisibleAgents,
  resolveWorkspaceVisibleTranscriptAgentId,
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
import { createTranscriptTurnExpansionController } from "./transcript-turn-expansion-controller.js"
import {
  buildEmptyTranscriptRenderable,
  buildLoadingTranscriptRenderable,
  buildNoSessionRenderable,
  buildWorkflowOutlineRenderable,
} from "./workspace-renderables.js"
import parserConfig from "./parsers-config.js"

const PROMPT_KEYBINDINGS = [
  { name: "return", action: "submit" },
  { name: "return", shift: true, action: "newline" },
  { name: "return", meta: true, action: "newline" },
] satisfies KeyBinding[]

const LIVE_TRANSCRIPT_LIMIT = 400
const LIVE_TRANSCRIPT_MAX_CHARS = 250_000
const STREAM_BATCH_WINDOW_MS = 48
const CHROME_UPDATE_THROTTLE_MS = 48
const TURN_COMPLETION_QUIET_MS = 1_500
const COMMAND_CENTER_OVERLAY_FOOTPRINT = 3
const ATTACHED_PROMPT_PLACEHOLDER = "Write your next prompt here"

type PromptQueueItem = {
  id: string
  source_attachment_id: string
  target_agent_id?: string | null
  prompt: string
  status: string
}

const DEBUG_LOGS_ENABLED = (process.env.ARROBA_LOG_LEVEL ?? "").toLowerCase() === "debug"
const OPEN_CONSOLE_ON_ERROR = process.env.ARROBA_OPEN_CONSOLE_ON_ERROR === "1"
let processLogger: ArrobaLogger | null = null
let transcriptParsersRegistered = false

function getLogger(component: string, fields: Record<string, unknown> = {}) {
  return processLogger?.child(component, fields) ?? null
}

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

  ensureTranscriptParsersRegistered()
  processLogger = createProcessLogger("cli")
  getLogger("cli.main")?.info("starting cli process", { argv })
  const options = parseArgs(argv)
  const preferences = await loadPreferences()
  applyProviderPreferenceDefaults(options, preferences)
  const kernelEndpoint = options.relayUrl ?? options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint()
  const client = new LocalIpcClient(kernelEndpoint, options.relayUrl
    ? {
      relayAuthToken: options.relayToken,
      targetDaemonId: options.targetDaemonId,
      targetDaemonAlias: options.targetDaemonAlias,
    }
    : undefined)
  const inferredTargets = await inferWorkspaceTargetsFromLaunchDirectory(process.cwd())
  const workspace = options.workspace ?? inferredTargets.workspace
  const worktree = options.worktree ?? inferredTargets.worktree
  await primeWaitingRoomWorktreeInventory({
    cwd: process.cwd(),
    workspacePath: workspace,
    currentWorktreePath: worktree,
  })
  const themeRegistry = await loadThemeRegistry({
    workspace,
    onWarning: (warning) => {
      getLogger("cli.main")?.warn("skipping custom theme", warning)
    },
  })
  if (options.deleteSessionRef) {
    await deleteSessionByRef(client, options.deleteSessionRef, workspace)
    return
  }
  getLogger("cli.main")?.info("bootstrapping cli session", {
    kernel_endpoint: kernelEndpoint,
    workspace_id: workspace,
    worktree_id: worktree,
    client_id: options.clientId,
  })
  if (!options.detached && isNoArgDefaultKernelLaunch(argv) && !(await isKernelEndpointReachable(kernelEndpoint))) {
    getLogger("cli.main")?.warn("default local kernel unavailable; launching detached waiting room", {
      kernel_endpoint: kernelEndpoint,
    })
    options.detached = true
  }
  let bootstrap: BootstrapState
  if (options.detached) {
    bootstrap = buildDetachedBootstrap(client, options, preferences)
  } else {
    try {
      bootstrap = await bootstrapSession(client, options, workspace, worktree, preferences, {
        logger: getLogger("cli.main"),
        listSessions,
        getProviderCatalog,
        getProviderCommandCatalogs,
        createSession,
        resolveSession,
        attachToSession,
        getSessionState,
        launchProviderRun,
        tryGetProviderRun,
        catchUpAttachedSession,
        getSessionHistory,
        getPromptInputHistory,
        resolveVisibleAgentId: (session, nextPreferences) => {
          const focusedAgentId = session.focused_agent_id ?? session.agents[0]?.id ?? null
          return selectResponsePaneAgents(
            session.agents,
            focusedAgentId,
            sessionResponseLayout(session, nextPreferences.ui?.multiAgentResponseLayout) === "split",
            resolveMaxAgentsPerScreen(nextPreferences.ui?.maxAgentsPerScreen),
          ).visibleTranscriptAgentId
        },
        prepareHistoryEntries: (entries, session) =>
          reindexTranscriptEntries(
            hydrateTranscriptEntries(entries),
            0,
          ),
      })
    } catch (error) {
      if (!isNoArgDefaultKernelLaunch(argv) || !isKernelEndpointUnavailableError(error)) {
        throw error
      }
      getLogger("cli.main")?.warn("default local kernel unavailable; launching detached waiting room", {
        kernel_endpoint: kernelEndpoint,
        error: formatError(error),
      })
      options.detached = true
      bootstrap = buildDetachedBootstrap(client, options, preferences)
    }
  }
  bootstrap.themeRegistry = themeRegistry
  if (bootstrap.binding) {
    getLogger("cli.main")?.info("bootstrapped cli session", {
      session_id: bootstrap.binding.session.id,
      attachment_id: bootstrap.binding.attachment.id,
      created_session: bootstrap.binding.createdSession,
    })
    await maybeResize(client, bootstrap.binding.session.id)
  } else {
    getLogger("cli.main")?.info("starting cli without attached session")
  }
  await render(
    () => <ArrobaCliApp bootstrap={bootstrap} />, 
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

function buildDetachedBootstrap(
  client: LocalIpcClient,
  options: CliOptions,
  preferences: BootstrapState["preferences"],
): BootstrapState {
  return {
    client,
    binding: null,
    sessions: [],
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    options,
    preferences,
  }
}

function ensureTranscriptParsersRegistered() {
  if (transcriptParsersRegistered) {
    return
  }
  addDefaultParsers(parserConfig.parsers)
  transcriptParsersRegistered = true
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
  const hiddenWaitingRoomKernelIds = new Set<string>(initialPreferences.ui?.hiddenRemoteKernelIds ?? [])
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
  let promptInput: TextareaRenderable | undefined
  let transcriptScrollbox: ScrollBoxRenderable | undefined
  let responseLayoutBox: BoxRenderable | undefined
  const responseRowBoxes: Array<BoxRenderable | undefined> = []
  const paneGridBorderRows: Array<BoxRenderable | undefined> = []
  let paneGridBottomBorderRow: BoxRenderable | undefined
  const paneGridHorizontalSegments: Array<Array<BoxRenderable | undefined>> = []
  const paneGridBottomHorizontalSegments: Array<BoxRenderable | undefined> = []
  const paneGridJunctionTexts: Array<Array<TextRenderable | undefined>> = []
  const paneGridBottomJunctionTexts: Array<TextRenderable | undefined> = []
  const paneGridVerticalSegments: Array<Array<BoxRenderable | undefined>> = []
  let responsePrimaryPane: BoxRenderable | undefined
  const responseAuxiliaryPanes: Array<BoxRenderable | undefined> = []
  const responseAuxiliaryScrollboxes: Array<ScrollBoxRenderable | undefined> = []
  let responsePrimaryInteractionBox: BoxRenderable | undefined
  const responseAuxiliaryInteractionBoxes: Array<BoxRenderable | undefined> = []
  let responsePrimaryFooterBox: BoxRenderable | undefined
  const responseAuxiliaryFooterBoxes: Array<BoxRenderable | undefined> = []
  const splitPaneFooterRenderState = createSplitPaneFooterRenderState()
  const responseAuxiliaryAgentIds: Array<string | null> = []
  const interactionChoiceSelection = new Map<string, number>()
  const interactionCustomReplies = new Map<string, string>()
  const interactionCustomEditing = new Set<string>()
  const agentTranscriptScrollboxes = new Map<string, ScrollBoxRenderable>()
  const agentTranscriptRenderables = new Map<string, Map<number, TranscriptEntryRenderable>>()
  const agentEmptyTranscriptRenderables = new Map<string, BoxRenderable>()
  const agentPaneTools = new Map<string, Map<string, ToolTranscriptUpdate>>()
  let promptStateBox: BoxRenderable | undefined
  let statusIndicatorBox: BoxRenderable | undefined
  let footerSummaryBox: BoxRenderable | undefined
  let historyLoadingBox: BoxRenderable | undefined
  let promptMetaProviderText: TextRenderable | undefined
  let promptMetaProviderDividerText: TextRenderable | undefined
  let promptMetaModelText: TextRenderable | undefined
  let promptMetaModelDividerText: TextRenderable | undefined
  let promptMetaVariantText: TextRenderable | undefined
  let promptMetaUsageDividerText: TextRenderable | undefined
  let promptMetaUsageTokensText: TextRenderable | undefined
  let promptMetaUsageBarOpenText: TextRenderable | undefined
  let promptMetaUsageBarFilledText: TextRenderable | undefined
  let promptMetaUsageBarEmptyText: TextRenderable | undefined
  let promptMetaUsageBarCloseText: TextRenderable | undefined
  let promptMetaUsagePercentText: TextRenderable | undefined
  let commandCenterBox: BoxRenderable | undefined
  let hotkeysOverlayBox: BoxRenderable | undefined
  const sessionChromeSummaryRenderState = createSessionChromeSummaryRenderState()
  let historyLoadingText: TextRenderable | undefined
  const statusIndicatorRenderState = createStatusIndicatorRenderState()
  let closing = false
  const tools = new Map<string, ToolTranscriptUpdate>()
  const activeToolLabels = new Map<string, string>()
  const transcriptRenderables = new Map<number, TranscriptEntryRenderable>()
  let transcriptSyntax = createTranscriptSyntaxStyle()
  let emptyTranscriptRenderable: BoxRenderable | undefined
  let lastTranscriptScrollTop = 0
  const historyScrollRestoreController = createHistoryScrollRestoreController({
    scheduleTimer: (callback, delayMs) => {
      startTimeout(callback, delayMs)
    },
    getScrollbox: () => transcriptScrollbox,
    setLastScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
  })
  let uiBatchDepth = 0
  // Connection resilience tracking
  const SILENT_POLL_THRESHOLD = 8 // ~2 seconds of no activity (8 * 250ms polling interval)
  let lastLoggedFocusedBadgeState: string | null = null
  const agentFocusTransitionController = createAgentFocusTransitionController()
  let currentTurnId = computeCurrentTurnId(initialEntries)
  let nextTurnId = computeNextTurnId(initialEntries)
  let mountedTranscriptAgentId = initialBinding ? initialSession.focused_agent_id ?? initialSession.agents[0]?.id ?? null : null
  let submittingAgentId: string | null = null
  const promptTextController = createPromptTextController({
    initialText: initialPromptDraft,
    getPromptInput: () => promptInput ?? null,
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
    isBatched: () => uiBatchDepth > 0,
    getRenderable: () => transcriptScrollbox,
    requestRender: (renderable) => {
      renderScheduler.requestRenderable(renderable)
    },
  })

  const isAttached = () => attachmentState() !== null
  const focusedAgentId = () => sessionState().focused_agent_id ?? sessionState().agents[0]?.id ?? null
  const multiAgentMode = () => isAttached() && sessionState().agents.length > 1
  const workflowScreenShowing = () => isAttached() && workspaceScreenMode() === "workflow"
  const splitAgentResponseMode = () => isAttached() && sessionState().agents.length > 1 && multiAgentResponseLayout() === "split"
  const activeInteractionForAgent = (agentId: string | null | undefined): RuntimeInteraction | null => {
    if (!agentId) {
      return null
    }
    return sessionState().active_interactions?.find((interaction) => interaction.agent_id === agentId) ?? null
  }
  const focusedAgentInteraction = () => activeInteractionForAgent(focusedAgentId())
  const workflowPromptState = createMemo(() => deriveWorkflowPromptState({
    workflowScreenActive: workflowScreenShowing(),
    workflows: sessionState().workflows ?? [],
    workflowRuns: sessionState().workflow_runs ?? [],
    selectedWorkflowId: selectedWorkflowId(),
    selectedWorkflowNodeId: selectedWorkflowNodeId(),
  }))
  const responsePaneSelection = () => selectResponsePaneAgents(
    sessionState().agents,
    focusedAgentId(),
    splitAgentResponseMode(),
    maxAgentsPerScreen(),
  )
  const responsePaneAgentSignature = () => sessionState().agents.map((agent) => agent.id).join(",")
  const responsePrimaryAgent = () => workflowScreenActive() ? null : responsePaneSelection().primary
  const responseVisibleAgents = () => resolveWorkspaceVisibleAgents(workspaceScreenMode(), responsePaneSelection().visibleAgents)
  const visibleTranscriptAgentId = () => resolveWorkspaceVisibleTranscriptAgentId(
    workspaceScreenMode(),
    responsePaneSelection().visibleTranscriptAgentId,
  )
  const responsePaneRows = () => responsePaneRowSlots(maxAgentsPerScreen())
  createEffect(() => {
    if (!isAttached()) {
      return
    }
    const session = sessionState()
    setWorkspaceShellContext((previous) =>
      deriveWorkspaceShellContextForSession(previous, session, attachmentState()?.id))
  })
  const primaryTranscriptSurfaceTone = () => resolveTranscriptSurfaceTone(splitAgentResponseMode(), responsePrimaryAgent()?.id === focusedAgentId())
  const auxiliaryTranscriptSurfaceTone = (agentId: string | null | undefined) => {
    return resolveTranscriptSurfaceTone(splitAgentResponseMode(), Boolean(agentId) && agentId === focusedAgentId())
  }
  const scheduleResponsePaneRepaint = () => {
    renderScheduler.requestTree(responseLayoutBox)
    renderScheduler.requestTree(historyLoadingBox)
    renderScheduler.requestRoot()
  }
  const shouldRefreshAgentPanesForSessionChange = (nextSession: RuntimeSession) => {
    const previousAgentSignature = sessionState().agents.map((agent) => agent.id).join(",")
    const nextAgentSignature = nextSession.agents.map((agent) => agent.id).join(",")
    if (nextAgentSignature !== previousAgentSignature) {
      return true
    }
    if (splitAgentResponseMode()) {
      return false
    }
    return nextSession.focused_agent_id !== focusedAgentId()
  }

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
  const agentPanePreview = (agentId: string) => agentPanePreviews()[agentId] ?? ""
  const agentActivityLabel = (agentId: string | null | undefined) => (agentId ? agentActivityLabels()[agentId] ?? null : null)
  const focusedAgent = () => sessionState().agents.find((agent) => agent.id === focusedAgentId()) ?? null
  const focusedBackendProvider = (): BackendProviderId | null => {
    const provider = focusedAgent()?.provider
    return provider && isBackendProviderId(provider) ? provider : null
  }
  const focusedProviderRun = () => {
    const run = providerRunState()
    const agentId = focusedAgentId()
    return run && run.agent_instance_id === agentId ? run : null
  }
  const resolveSessionAgent = (reference?: string | null) => {
    return resolveSessionAgentReference(sessionState(), focusedAgentId(), reference)
  }
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
  const promptStateForAgent = (agentId: string | null | undefined) => agentPromptState(sessionState(), agentId)
  const agentQueuedDepth = (agentId: string | null | undefined) => promptStateForAgent(agentId)?.queued_prompts.length ?? 0
  const agentActivePrompt = (agentId: string | null | undefined) => promptStateForAgent(agentId)?.active_prompt ?? null
  const agentBusyLatch = (agentId: string | null | undefined) => readAgentBusyLatch(agentBusyLatches(), agentId)
  const anyPromptWork = () => sessionHasPromptWork(sessionState())
  const hasPromptWorkByAgent = () => {
    const state: Record<string, boolean> = {}
    for (const agent of sessionState().agents) {
      state[agent.id] = agentHasPromptWork(sessionState(), agent.id)
    }
    return state
  }
  const focusedPromptState = () => promptStateForAgent(focusedAgentId())
  const focusedQueueDepth = () => agentQueuedDepth(focusedAgentId())
  const focusedActivePrompt = () => agentActivePrompt(focusedAgentId())
  const activeToolLabelForAgent = (agentId: string | null | undefined) => {
    return resolveActiveToolLabelForAgent({
      agentId,
      visibleTranscriptAgentId: visibleTranscriptAgentId(),
      activeToolLabels: activeToolLabels.values(),
      agentPaneToolUpdates: agentId ? agentPaneTools.get(agentId)?.values() : null,
    })
  }
  const focusedActivityLabel = () => {
    const agentId = focusedAgentId()
    const toolLabel = activeToolLabelForAgent(agentId)
    return deriveFocusedActivityLabel({
      focusedAgentId: agentId,
      activeToolLabel: toolLabel,
      agentActivityLabel: agentActivityLabel(agentId),
    })
  }
  const setAgentBusyLatch = (agentId: string | null | undefined, busy: boolean) => {
    setAgentBusyLatches((current) => nextAgentBusyLatches(current, agentId, busy))
  }
  const markAgentBusy = (agentId: string | null | undefined) => {
    setAgentBusyLatch(agentId, true)
  }
  const clearAgentBusy = (agentId: string | null | undefined) => {
    setAgentBusyLatch(agentId, false)
  }
  const focusedAgentBusy = () => {
    return deriveFocusedAgentBusy({
      focusedAgentId: focusedAgentId(),
      submitting: submitting(),
      submittingAgentId,
      session: sessionState(),
      streamingAgentId: streamingAgentId(),
      focusedActivityLabel: focusedActivityLabel(),
      agentBusyLatches: agentBusyLatches(),
    })
  }
  const allAgentsBusyState = () => {
    return deriveAllAgentsBusyState({
      submitting: submitting(),
      submittingAgentId,
      session: sessionState(),
      streamingAgentId: streamingAgentId(),
      agentActivityLabels: agentActivityLabels(),
      agentBusyLatches: agentBusyLatches(),
    })
  }
  const shouldPreserveAgentActivityLabel = (agentId: string | null | undefined) => {
    return shouldPreserveAgentActivityLabelState({
      agentId,
      session: sessionState(),
      streamingAgentId: streamingAgentId(),
    })
  }
  const setAgentActivityLabel = (agentId: string | null | undefined, nextLabel: string | null) => {
    setAgentActivityLabels((current) => nextAgentActivityLabels(
      current,
      agentId,
      nextLabel,
      shouldPreserveAgentActivityLabel(agentId),
    ))
  }
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
  const logProviderRunDebug = (message: string, run: RuntimeProviderRun | null, fields: Record<string, unknown> = {}) => {
    appLogger?.debug(message, {
      provider_run_id: run?.id ?? null,
      provider: run?.provider ?? null,
      provider_model: run?.model ?? null,
      provider_variant: run?.variant ?? null,
      provider_usage_tokens_total: run?.usage_tokens_total ?? null,
      provider_state: run?.state ?? null,
      ...fields,
    })
  }
  const logViewDebug = (phase: string, fields: Record<string, unknown> = {}) => {
    if (!DEBUG_LOGS_ENABLED) {
      return
    }
    appLogger?.debug(`view debug: ${phase}`, {
      layout: multiAgentResponseLayout(),
      layout_is_split: multiAgentResponseLayout() === "split",
      split_active: splitAgentResponseMode(),
      attached: isAttached(),
      center_mode: "transcript",
      agent_count: sessionState().agents.length,
      focused_agent_id: focusedAgentId(),
      has_transcript_scrollbox: Boolean(transcriptScrollbox),
      ...fields,
    })
  }
  const logVisibleTranscriptOutput = (
    role: TranscriptEntry["role"],
    text: string,
    merged: boolean,
    mergeKey?: string,
  ) => {
    if (!["assistant", "reasoning", "tool", "error", "status"].includes(role)) {
      return
    }
    appLogger?.info("applied visible transcript output", {
      role,
      merged,
      merge_key: mergeKey ?? null,
      focused_agent_id: focusedAgentId(),
      visible_agent_id: visibleTranscriptAgentId(),
      preview: text.replace(/\s+/g, " ").trim().slice(0, 160),
    })
  }
  const logFocusedBadgeChange = (badge: FocusedStatusBadge) => {
    const nextState = `${badge.label}:${badge.parts.map((part) => `${part.label}:${part.tone}`).join("|")}`
    if (lastLoggedFocusedBadgeState === nextState) {
      return
    }
    lastLoggedFocusedBadgeState = nextState
    appLogger?.info("focused status badge changed", {
      label: badge.label,
      tone: badge.tone,
      parts: badge.parts,
      focused_agent_id: focusedAgentId(),
      visible_agent_id: visibleTranscriptAgentId(),
    })
  }
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
      transcriptSyntax = createTranscriptSyntaxStyle()
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
      promptInput?.focus()
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
    isKernelHidden: (kernelId) => hiddenWaitingRoomKernelIds.has(kernelId),
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
    hideRemoteKernel: (kernelId) => {
      hiddenWaitingRoomKernelIds.add(kernelId)
      const hiddenKernelIds = [...hiddenWaitingRoomKernelIds].sort()
      void saveUiPreferences({ hiddenRemoteKernelIds: hiddenKernelIds })
      setPreferencesState((current) => mergeUiPreferences(current, { hiddenRemoteKernelIds: hiddenKernelIds }))
    },
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
  const connectDetachedKernelFromWaitingRoom = async () => {
    appLogger?.info("connecting detached cli to configured kernel endpoint")
    flashFooter("connecting to kernel...", "info")
    const [catalog, commandCatalogs] = await Promise.all([
      getProviderCatalog(client, appLogger),
      getProviderCommandCatalogs(client, appLogger),
    ])
    waitingRoomInventoryRefreshController.invalidate()
    setProviderCatalogState(catalog)
    setProviderCommandCatalogState(commandCatalogs)
    setKernelConnected(true)
    setDaemonDisconnected(false)
    await refreshWaitingRoomData()
    flashFooter("connected to kernel", "info")
  }
  const refreshWaitingRoomDataNow = waitingRoomInventoryRefreshController.refreshNow
  const refreshWaitingRoomData = waitingRoomInventoryRefreshController.refresh
  const currentProviderSelection = () => deriveCurrentProviderSelection({
    providerRun: focusedProviderRun(),
    focusedAgent: focusedAgent(),
    waitingRoomState: waitingRoomState(),
    defaultProvider: options.provider ?? "opencode",
    defaultModel: options.model,
    defaultEffort: options.effort,
  })
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
  const promptMetaParts = (): PromptMetaPart[] => derivePromptMetaState({
    providerRun: focusedProviderRun(),
    focusedAgent: focusedAgent(),
    waitingRoomState: waitingRoomState(),
    defaultProvider: options.provider ?? "opencode",
    defaultModel: options.model,
    defaultEffort: options.effort,
  })
  const promptUsageMeta = () => derivePromptUsageState({
    providerRun: focusedProviderRun(),
    catalog: providerCatalogState(),
  })
  const currentModelId = () => currentProviderSelection().model
  const currentVariantId = () => currentProviderSelection().effort
  const waitingRoomTargets = () => ({
    workspacePath: pendingWorkspaceTarget(),
    worktreePath: pendingWorktreeTarget(),
  })
  const commandCenterVisibleRowCount = () => Math.max(4, Math.min(10, dimensions().height - (promptInput?.height ?? 1) - 10))
  const commandCenterController = createCommandCenterController({
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
    render: (state) => {
      renderCommandCenterOverlay({
        box: commandCenterBox,
        renderer,
        open: state.open,
        items: state.items,
        selectedIndex: state.selectedIndex,
        visibleRowCount: commandCenterVisibleRowCount(),
        promptHeight: promptInput?.height ?? 1,
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
        activeToolLabels.clear()
        setAgentActivityLabels({})
        setStreamingAgentId(null)
        setSubmitting(false)
        submittingAgentId = null
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
  const sessionStatusMode = (): SessionStatusMode => {
    return deriveSessionStatusMode({
      daemonDisconnected: daemonDisconnected(),
      working: working(),
      hasActivePrompt: anyPromptWork(),
      submitting: submitting(),
      queueDepth: focusedQueueDepth(),
    })
  }
  const footerHint = () => {
    return deriveFooterHint({
      fatalError: fatalError(),
      activePromptId: focusedActivePrompt()?.id ?? null,
      queueDepth: focusedQueueDepth(),
      statusLine: statusLine(),
    })
  }
  const promptPlaceholder = () => {
    return derivePromptPlaceholder({
      attached: isAttached(),
      workflowScreenActive: workflowScreenShowing(),
      workflowPromptState: workflowPromptState(),
      attachedPlaceholder: ATTACHED_PROMPT_PLACEHOLDER,
      detachedPlaceholder: SESSION_NEW_PLACEHOLDER,
    })
  }
  const promptAreaBackground = () => {
    themeRevision()
    return derivePromptAreaBackground({
      attached: isAttached(),
      workflowScreenActive: workflowScreenShowing(),
      attachedBackground: theme.backgroundPanel,
      detachedBackground: theme.backgroundElement,
      workflowBackground: theme.backgroundElement,
    })
  }
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
  const persistSessionPromptState = async (
    sessionId: string,
    next: {
      promptHistory?: readonly string[]
      promptDraft?: string | null
    },
  ) => {
    setPreferencesState((current) => mergeSessionPromptState(current, sessionId, next))
    await saveSessionPromptState(sessionId, next)
  }
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
  const scheduleSharedPromptInputHistoryRefresh = () => {
    const sessionId = attachmentState()?.session_id
    if (!sessionId) {
      return
    }
    promptInputHistoryRefreshController.schedule(sessionId)
  }
  const persistablePromptDraft = () => {
    if (promptHistoryDraft() !== null) {
      return promptHistoryDraft() ?? ""
    }
    return promptTextController.currentText()
  }
  const recordPromptAreaHistoryEntry = promptInputHistoryController.recordPromptAreaEntry
  const syncPromptTextSnapshot = promptTextController.syncSnapshot
  const promptAttachmentHighlightController = createPromptAttachmentHighlightController({
    getPromptInput: () => promptInput ?? null,
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
      promptInput?.focus()
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
    getPromptInput: () => promptInput ?? null,
    getPlaceholder: promptPlaceholder,
  })
  const syncPromptPlaceholder = promptPlaceholderSyncController.sync
  createEffect(() => {
    promptPlaceholder()
    syncPromptPlaceholder()
  })
  const promptAttachmentController = createPromptAttachmentController({
    getPromptInput: () => promptInput ?? null,
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
  const hotkeySections = () => buildHotkeySections(isAttached())
  const sessionBrowserSessions = () => sessionBrowserVisibleSessions(availableSessions())
  const normalizeSessionBrowserIndex = () => {
    const sessions = sessionBrowserSessions()
    const index = clampSessionBrowserIndex(sessionBrowserIndex(), sessions.length)
    if (index !== sessionBrowserIndex()) {
      setSessionBrowserIndex(index)
    }
    return index
  }
  const dialogOverlayController = createCliDialogOverlayController({
    getOpenState: () => ({
      hotkeysOpen: hotkeysOpen(),
      terminalPairingOpen: terminalPairingOpen(),
      sessionBrowserOpen: sessionBrowserOpen(),
    }),
    getCurrentFocus: () => currentFocusedRenderable() as CliDialogFocusTarget | null,
    getPromptFocus: () => promptInput as CliDialogFocusTarget | null | undefined,
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
    renderOverlay: (mode, onDismiss) => {
      renderCliDialogOverlay({
        overlayBox: hotkeysOverlayBox,
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
    getPromptText: () => promptInput ? promptTextController.currentText() : null,
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
    currentTurnId: () => currentTurnId,
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
    getScrollbox: () => transcriptScrollbox,
    cancelHistoryScrollRestore: () => historyScrollRestoreController.cancel(),
    setLastTranscriptScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
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
    nextTurnId: () => nextTurnId,
    setNextTurnId: (turnId) => {
      nextTurnId = turnId
    },
    setCurrentTurnId: (turnId) => {
      currentTurnId = turnId
    },
    setSubmittingAgentId: (agentId) => {
      submittingAgentId = agentId
    },
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
    renderables: transcriptRenderables,
    removeFromScrollbox: (renderableId) => {
      if (!transcriptScrollbox) {
        return false
      }
      transcriptScrollbox.remove(renderableId)
      return true
    },
    requestScrollboxRender: () => {
      transcriptScrollbox?.requestRender()
    },
    deleteTool: (mergeKey) => {
      tools.delete(mergeKey)
    },
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

  const hotkeyDebug = (message: string) => {
    appLogger?.debug("hotkeys footer debug", { detail: message })
    if (!DEBUG_LOGS_ENABLED) {
      return
    }
    flashFooter(`[hotkeys] ${message}`, "info")
  }
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
    promptInput: () => promptInput ?? null,
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
    clearSubmittingAgentId: () => {
      submittingAgentId = null
    },
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
    clearActiveToolLabels: () => {
      activeToolLabels.clear()
    },
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
    clearActiveToolLabels: () => {
      activeToolLabels.clear()
    },
    setAgentActivityLabels,
    setStreamingAgentId,
    setSubmitting,
    clearSubmittingAgentId: () => {
      submittingAgentId = null
    },
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

  const syncVisibleActivityLabel = () => {
    setActiveStatusLabel(focusedActivityLabel())
  }

  const transcriptStreamController = createTranscriptStreamController({
    entries: () => entries,
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    entryCounter,
    currentTurnId: () => currentTurnId,
    tools,
    activeToolLabels,
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

  const renderSplitPaneFooters = () => {
    renderSplitPaneFootersView({
      renderer,
      state: splitPaneFooterRenderState,
      primaryBox: responsePrimaryFooterBox,
      auxiliaryBoxes: responseAuxiliaryFooterBoxes,
      showAgentFooters: isAttached() && !workflowScreenActive() && responseVisibleAgents().length > 0,
      maxAgentsPerScreen: maxAgentsPerScreen(),
      visibleAgents: responseVisibleAgents(),
      focusedAgentId: focusedAgentId(),
      providerRun: providerRunState(),
      currentProviderSelection: currentProviderSelection(),
      agentActivityLabels: agentActivityLabels(),
      hasPromptWorkByAgent: hasPromptWorkByAgent(),
      streamingAgentId: streamingAgentId(),
      agentBusyLatch,
      sessionConfigValues: sessionState().config_state?.values,
      agentLocationLabel,
      badgeWidth: STATUS_BADGE_WIDTH,
      animationFrame: workingAnimationFrame(),
    })
  }

  const renderAgentInteractions = () => {
    renderAgentInteractionStrips({
      renderer,
      primaryBox: responsePrimaryInteractionBox,
      auxiliaryBoxes: responseAuxiliaryInteractionBoxes,
      visibleAgents: responseVisibleAgents(),
      maxAgentsPerScreen: maxAgentsPerScreen(),
      focusedAgentId: focusedAgentId(),
      activeInteractionForAgent,
      selectedChoiceIndex: (interactionId) => interactionChoiceSelection.get(interactionId) ?? 0,
      setSelectedChoiceIndex: (interactionId, index) => {
        interactionChoiceSelection.set(interactionId, index)
      },
      customReply: (interactionId) => interactionCustomReplies.get(interactionId) ?? "",
      customEditing: (interactionId) => interactionCustomEditing.has(interactionId),
    })
  }

  const setPromptMetaRenderables = (parts: PromptMetaPart[]) => {
    renderPromptMeta({
      providerText: promptMetaProviderText,
      providerDividerText: promptMetaProviderDividerText,
      modelText: promptMetaModelText,
      modelDividerText: promptMetaModelDividerText,
      variantText: promptMetaVariantText,
      usageDividerText: promptMetaUsageDividerText,
      usageTokensText: promptMetaUsageTokensText,
      usageBarOpenText: promptMetaUsageBarOpenText,
      usageBarFilledText: promptMetaUsageBarFilledText,
      usageBarEmptyText: promptMetaUsageBarEmptyText,
      usageBarCloseText: promptMetaUsageBarCloseText,
      usagePercentText: promptMetaUsagePercentText,
    }, parts, promptUsageMeta())
  }

  const renderHistoryLoadingIndicator = () => {
    renderHistoryLoadingIndicatorView({
      box: historyLoadingBox,
      text: historyLoadingText,
      loading: loadingHistory(),
      renderer,
      assignText: (value) => {
        historyLoadingText = value
      },
    })
  }

  const setHistoryLoadingState = (next: boolean) => {
    setLoadingHistory(next)
    renderHistoryLoadingIndicator()
  }

  const setSessionHydratingState = (next: boolean) => {
    if (sessionHydrating() === next) {
      return
    }
    setSessionHydrating(next)
    if (isAttached() && visibleTranscriptEntries().length === 0 && !workflowScreenActive()) {
      rebuildTranscript()
      return
    }
    requestTranscriptRender()
  }

  const requestTranscriptRender = () => {
    transcriptRenderDeferralController.request()
  }

  const flushDeferredUiUpdates = () => {
    transcriptRenderDeferralController.flush()
    sessionChromeUpdateController.flushDeferred()
  }

  const runUiBatch = (callback: () => void) => {
    uiBatchDepth += 1
    batch(callback)
    uiBatchDepth -= 1
    if (uiBatchDepth === 0) {
      flushDeferredUiUpdates()
    }
  }

  const renderStatusIndicator = () => {
    const attached = isAttached()
    const badge = attached ? focusedStatusBadge() : null
    if (!badge) {
      lastLoggedFocusedBadgeState = null
    } else {
      logFocusedBadgeChange(badge)
    }
    renderStatusIndicatorView({
      renderer,
      box: statusIndicatorBox,
      state: statusIndicatorRenderState,
      attached,
      badge,
      badgeWidth: STATUS_BADGE_WIDTH,
      animationFrame: workingAnimationFrame(),
    })
  }

  const applyResponseLayout = () => {
    const primaryPane = responsePrimaryPane
    const split = splitAgentResponseMode()
    const visibleAgents = responseVisibleAgents()
    const paneRows = responsePaneRows()
    const showWorkflowScreen = workflowScreenActive()
    const paneGrid = buildPaneGridModel({
      paneRows,
      visibleAgents,
      focusedAgentId: focusedAgentId(),
      split,
      showWorkflowScreen,
    })

    const appliedPaneLayout = applyResponsePaneGridLayout({
      layoutBox: responseLayoutBox,
      primaryPane,
      primaryInteractionBox: responsePrimaryInteractionBox,
      primaryFooterBox: responsePrimaryFooterBox,
      primaryScrollbox: transcriptScrollbox,
      historyLoadingBox,
      auxiliaryPanes: responseAuxiliaryPanes,
      auxiliaryInteractionBoxes: responseAuxiliaryInteractionBoxes,
      auxiliaryFooterBoxes: responseAuxiliaryFooterBoxes,
      auxiliaryScrollboxes: responseAuxiliaryScrollboxes,
      rowBoxes: responseRowBoxes,
      borderRows: paneGridBorderRows,
      horizontalSegments: paneGridHorizontalSegments,
      verticalSegments: paneGridVerticalSegments,
      junctionTexts: paneGridJunctionTexts,
      bottomBorderRow: paneGridBottomBorderRow,
      bottomHorizontalSegments: paneGridBottomHorizontalSegments,
      bottomJunctionTexts: paneGridBottomJunctionTexts,
      paneRows,
      paneGrid,
      split,
      showWorkflowScreen,
      theme,
      emptyTextAttributes: TextAttributes.NONE,
      panelBackgroundForFocus: (focused) => transcriptSurfacePalette(resolveTranscriptSurfaceTone(true, focused)).panel,
      onMissingRefs: (details) => {
        logViewDebug("apply response layout:missing refs", {
          has_layout_box: details.hasLayoutBox,
          has_primary_pane: details.hasPrimaryPane,
          auxiliary_pane_count: details.auxiliaryPaneCount,
        })
      },
    })
    if (!appliedPaneLayout) {
      return
    }

    renderSplitPaneFooters()
    renderAgentInteractions()

    for (let auxiliaryIndex = 0; auxiliaryIndex < maxAgentsPerScreen() - 1; auxiliaryIndex += 1) {
      const paneIndex = auxiliaryIndex + 1
      syncAuxiliaryPane({
        scrollbox: responseAuxiliaryScrollboxes[auxiliaryIndex],
        nextAgentId: split ? (visibleAgents[auxiliaryIndex + 1]?.id ?? null) : null,
        currentAgentId: responseAuxiliaryAgentIds[auxiliaryIndex] ?? null,
        splitMode: split,
        clearAuxiliaryAgentPane,
        unregisterAgentScrollbox: (agentId) => {
          agentTranscriptScrollboxes.delete(agentId)
        },
        assignCurrentAgentId: (value) => {
          responseAuxiliaryAgentIds[auxiliaryIndex] = value
        },
        registerAgentScrollbox: (agentId, scrollbox) => {
          agentTranscriptScrollboxes.set(agentId, scrollbox)
        },
        rebuildAuxiliaryAgentPane,
        buildEmptyTranscriptRenderable: () => buildEmptyTranscriptRenderable(renderer),
      })
    }

    const nextVisibleTranscriptAgentId = responsePaneSelection().visibleTranscriptAgentId
    if (
      nextVisibleTranscriptAgentId
      && nextVisibleTranscriptAgentId !== mountedTranscriptAgentId
    ) {
      replaceTranscriptEntries(
        (agentPaneEntries()[nextVisibleTranscriptAgentId] ?? []).map((entry) => ({ ...entry })),
        nextVisibleTranscriptAgentId,
      )
    }

    scheduleResponsePaneRepaint()

    logViewDebug("apply response layout", {
      split,
      visible_agent_ids: visibleAgents.map((agent) => agent.id),
      screen_index: responsePaneSelection().screenIndex,
      screen_count: responsePaneSelection().screenCount,
    })
  }

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
    submittingAgentId = null
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

  const applySessionChromeUpdate = () => {
    syncPromptPlaceholder()
    renderSessionChromeSummary({
      renderer,
      state: sessionChromeSummaryRenderState,
      promptStateBox,
      footerSummaryBox,
      promptStateLabel: fatalError() ? "error" : submitting() ? "thinking" : footerHint(),
      promptStateTone: fatalError() ? "error" : submitting() ? "thinking" : "muted",
      footerSummary: isAttached()
        ? deriveAttachedFooterSummary({
            session: sessionState(),
            connectedClientCount: connectedClientCount(),
            multiAgentMode: multiAgentMode(),
            responseLayout: multiAgentResponseLayout(),
            sessionStatusMode: sessionStatusMode(),
            hotkeyToggleLabel: HOTKEY_TOGGLE_LABEL,
            focusedHasPromptWork: agentHasPromptWork(sessionState(), focusedAgentId()),
          })
        : SESSION_NEW_FOOTER_HINT,
      footerFlash: footerFlash(),
    })
    setPromptMetaRenderables(isAttached() ? promptMetaParts() : [])
    renderStatusIndicator()
    renderSplitPaneFooters()
    renderAgentInteractions()
  }

  const shouldThrottleSessionChrome = () => (
    working()
    || Boolean(activeStatusLabel())
    || Boolean(providerActivityLabel())
    || Boolean(streamingAgentId())
  )

  const sessionChromeUpdateController = createSessionChromeUpdateController({
    delayMs: CHROME_UPDATE_THROTTLE_MS,
    scheduleTimer: startTimeout,
    clearTimer: clearTimeout,
    isBatched: () => uiBatchDepth > 0,
    applyUpdate: applySessionChromeUpdate,
  })
  const renderSessionChromeBoundary = sessionChromeUpdateController.flush
  const updateSessionChrome = () => {
    sessionChromeUpdateController.request(shouldThrottleSessionChrome())
  }

  const setAgentPanePreview = (agentId: string, text: string) => {
    setAgentPanePreviews((current) => ({
      ...current,
      [agentId]: text,
    }))
  }

  const persistVisibleTranscriptEntries = (nextEntries: TranscriptEntry[]) => {
    const agentId = visibleTranscriptAgentId()
    if (!isAttached() || !agentId) {
      return
    }

    const persistedEntries = nextEntries.map((entry) => ({ ...entry }))
    setAgentPaneEntries((current) => ({
      ...current,
      [agentId]: persistedEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(persistedEntries))
  }

  const setAgentTranscriptEntries = (
    agentId: string,
    nextEntries: TranscriptEntry[],
    turnIds = expandedTurnIdsForAgent(agentId),
  ) => {
    const previousPaneEntries = agentPaneEntries()[agentId] ?? []
    const sanitizedEntries = applyTranscriptDisplayState(nextEntries.filter(Boolean), turnIds)
    commitAgentPaneEntries(agentId, sanitizedEntries)
    if (splitAgentResponseMode() && agentId === responsePrimaryAgent()?.id) {
      replaceTranscriptEntries(sanitizedEntries.map((entry) => ({ ...entry })), agentId)
    }
    if (splitAgentResponseMode() && visibleAuxiliaryAgentIds().includes(agentId)) {
      reconcileMountedAuxiliaryTranscript(agentId, previousPaneEntries, sanitizedEntries)
    }
  }

  const visibleAuxiliaryAgentIds = () => splitPaneAuxiliaryAgentIds(
    sessionState().agents,
    focusedAgentId(),
    true,
    maxAgentsPerScreen(),
  )

  const commitAgentPaneEntries = (agentId: string, nextEntries: TranscriptEntry[]) => {
    const persistedEntries = nextEntries.map((entry) => ({ ...entry }))
    setAgentPaneEntries((current) => ({
      ...current,
      [agentId]: persistedEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(persistedEntries))
  }

  const currentAgentPaneEntries = (agentId: string) => {
    return selectCurrentAgentPaneEntries({
      agentId,
      visibleAgentId: visibleTranscriptAgentId(),
      visibleEntries: entries.filter(Boolean),
      paneEntriesByAgent: agentPaneEntries(),
    })
  }

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
    scrollboxes: agentTranscriptScrollboxes,
    entryRenderables: agentTranscriptRenderables,
    emptyRenderables: agentEmptyTranscriptRenderables,
    toolStates: agentPaneTools,
    paneEntries: (agentId) => agentPaneEntries()[agentId] ?? [],
    buildEmptyRenderable: () => buildEmptyTranscriptRenderable(renderer),
    buildEntryRenderable: (agentId, entry) => buildTranscriptEntryRenderable(
      renderer,
      entry,
      transcriptSyntax,
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

  const trimLiveAgentPaneEntries = (agentId: string, nextEntries: TranscriptEntry[]) => trimAgentPaneEntries({
    entries: nextEntries,
    maxEntries: LIVE_TRANSCRIPT_LIMIT,
    maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
    onTrimmedMergeKey: (mergeKey) => {
      auxiliaryAgentPaneTools(agentId).delete(mergeKey)
    },
  })

  const commitStreamingAgentPaneEntry = (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
    updatedEntryId: number,
  ) => {
    const sanitizedEntries = applyTranscriptDisplayState(
      trimLiveAgentPaneEntries(agentId, nextEntries).filter(Boolean),
      expandedTurnIdsForAgent(agentId),
    )
    commitAgentPaneEntries(agentId, sanitizedEntries)
    if (splitAgentResponseMode() && agentId === responsePrimaryAgent()?.id) {
      replaceTranscriptEntries(sanitizedEntries.map((entry) => ({ ...entry })), agentId)
      return
    }
    if (!splitAgentResponseMode() || !visibleAuxiliaryAgentIds().includes(agentId)) {
      return
    }
    const updatedEntry = sanitizedEntries.find((entry) => entry.id === updatedEntryId)
    if (updatedEntry) {
      updateAuxiliaryTranscriptEntry(agentId, updatedEntry)
      return
    }
    reconcileMountedAuxiliaryTranscript(agentId, currentEntries, sanitizedEntries)
  }

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
    if (!agentId || agentId !== mountedTranscriptAgentId) {
      return
    }
    setAgentPaneEntries((current) => ({
      ...current,
      [agentId]: currentEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(currentEntries))
  })

  const refreshAgentPanes = async (session: RuntimeSession) => {
    const nextPaneState = await refreshAgentPaneState<AgentInstance, SessionHistoryPageEntry, TranscriptEntry, SessionHistoryCursor>({
      session,
      hasPromptWork: sessionHasPromptWork(session),
      expandedTurnIdsByAgent: expandedTurnIdsByAgent(),
      currentPaneEntriesByAgent: Object.fromEntries(
        session.agents.map((agent) => [agent.id, currentAgentPaneEntries(agent.id)]),
      ),
      resolveVisibleAgentId: (agents, focusedAgentId) =>
        selectResponsePaneAgents(
          agents,
          focusedAgentId,
          splitAgentResponseMode(),
          maxAgentsPerScreen(),
        ).visibleTranscriptAgentId,
      loadHistoryPage: async (agentId, cursor) => {
        const historyPage = await getSessionHistory(client, session.id, cursor, agentId)
        return {
          entries: historyPage.entries,
          nextCursor: historyPage.next_cursor,
        }
      },
      hydrateEntries: hydrateTranscriptEntries,
      stitchPrependedHistory,
      collapseHistoricalTurns: (entries) => entries,
      applyExpandedTurns,
      reindexEntries: reindexTranscriptEntries,
      formatPreview: formatTranscriptPreview,
      preserveExpandedTurnIds: true,
    })

    pruneAuxiliaryAgentPanes(session)
    setExpandedTurnIdsByAgent(nextPaneState.expandedTurnIdsByAgent)
    setAgentPanePreviews(nextPaneState.previews)
    setAgentPaneEntries(nextPaneState.paneEntries)
    setNextHistoryCursor(nextPaneState.visibleCursor)
    replaceTranscriptEntries(
      (nextPaneState.visibleAgentId ? nextPaneState.paneEntries[nextPaneState.visibleAgentId] : nextPaneState.visibleEntries)
        ?.map((entry) => ({ ...entry })) ?? [],
      nextPaneState.visibleAgentId,
    )
    applyResponseLayout()
    if (splitAgentResponseMode()) {
      for (const agentId of splitPaneAuxiliaryAgentIds(
        session.agents,
        session.focused_agent_id,
        true,
        maxAgentsPerScreen(),
      )) {
        rebuildAuxiliaryAgentPane(agentId)
      }
    }
  }

  const primaryTranscriptRenderController = createPrimaryTranscriptRenderController({
    getScrollbox: () => transcriptScrollbox,
    getEmptyRenderable: () => emptyTranscriptRenderable,
    setEmptyRenderable: (renderable) => {
      emptyTranscriptRenderable = renderable
    },
    renderables: transcriptRenderables,
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
      transcriptSyntax,
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
    setLastScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
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
      promptInput?.focus()
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
    getScrollbox: () => transcriptScrollbox,
    getEntries: () => entries.filter(Boolean),
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
    expandedTurnIdsForAgent,
    clearToolState: () => {
      tools.clear()
    },
    setEntries: (nextEntries) => {
      setEntries(reconcile(nextEntries))
    },
    setEntryCounter,
    setCurrentTurnId: (turnId) => {
      currentTurnId = turnId
    },
    setNextTurnId: (turnId) => {
      nextTurnId = turnId
    },
    setMountedTranscriptAgentId: (agentId) => {
      mountedTranscriptAgentId = agentId
    },
    setLastScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
    rebuildTranscript,
    syncVisibleTranscriptPreview,
    restorePrependedHistory: (request) => historyScrollRestoreController.restorePrependedHistory(request),
  })
  const replaceTranscriptEntries = primaryTranscriptEntryController.replaceEntries
  const prependTranscriptEntries = primaryTranscriptEntryController.prependEntries

  const clearAgentPaneRuntime = () => {
    agentPaneTranscriptRenderController.clearAll()
    responseAuxiliaryAgentIds.length = 0
  }

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
    getScrollbox: () => transcriptScrollbox,
    isScrollRestoring: () => historyScrollRestoreController.isRestoring(),
    isAttached,
    isLoadingHistory: loadingHistory,
    hasMoreHistory: () => nextHistoryCursor() !== null,
    getLastScrollTop: () => lastTranscriptScrollTop,
    setLastScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
    loadOlderHistory: async () => {
      const loaded = await transcriptHistoryLoadController.loadOlderPage()
      if (loaded) {
        scheduleShortViewportHistoryCheck()
      }
    },
  })
  function scheduleShortViewportHistoryCheck() {
    transcriptHistoryAutoloadController.scheduleShortViewportCheck()
  }

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
    isClosing: () => closing,
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
    clearActiveToolLabels: () => {
      activeToolLabels.clear()
    },
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
      promptInput?.clear()
    },
    syncPromptTextSnapshot,
    blurPromptInput: () => {
      promptInput?.blur()
    },
    focusPromptInput: () => {
      promptInput?.focus()
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
    scheduleShortViewportHistoryCheck: () => {
      scheduleShortViewportHistoryCheck()
    },
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

  const executeCommandCenterCommand = async (value: string) => {
    await executeSlashCommand(value, {
      onExit: requestExit,
      onWaiting: requestWaitingRoom,
      onStop: requestPromptStop,
      onAttachment: async (command) => {
        await handleAttachmentCommand(command.raw)
      },
      onSession: handleSessionCommand,
      onProvider: handleProviderCommand,
      onModel: handleModelCommand,
      onVariant: handleVariantCommand,
      onView: handleViewCommand,
      onAgent: async (command) => {
        try {
          await handleAgentCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onKernel: async (command) => {
        try {
          await handleKernelCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onMachine: async (command) => {
        try {
          await handleMachineCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onSlice: async (command) => {
        try {
          await handleSliceCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onRelay: async (command) => {
        try {
          await handleRelayCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onCloud: async (command) => {
        try {
          await handleCloudCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onConfig: async (command) => {
        try {
          await handleConfigCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onWorkspace: async (command) => {
        try {
          await handleWorkspaceCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onWorktree: async (command) => {
        try {
          await handleWorktreeCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onWorkflow: async (command) => {
        try {
          await handleWorkflowCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onMcp: async (command) => {
        try {
          await handleMcpCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onSkill: async (command) => {
        try {
          await handleSkillCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
    })
  }

  const exitController = createCliExitController({
    isClosing: () => closing,
    setClosing: (value) => {
      closing = value
    },
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
    isClosing: () => closing,
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
    clearActiveToolLabels: () => activeToolLabels.clear(),
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
    getSubmittingAgentId: () => submittingAgentId,
    clearAgentBusy,
    setSubmittingAgentId: (agentId) => {
      submittingAgentId = agentId
    },
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
    clearActiveToolLabels: () => activeToolLabels.clear(),
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
    getSubmittingAgentId: () => submittingAgentId,
    clearAgentBusy,
    setSubmittingAgentId: (agentId) => {
      submittingAgentId = agentId
    },
    setSubmitting,
    setFatalError,
    flashFooter,
    logInfo: (message, fields) => appLogger?.info(message, fields),
    logError: (message, fields) => appLogger?.error(message, fields),
    formatError,
  })

  const promptSubmitCoordinator = createPromptSubmitCoordinator({
    getPromptText: () => promptInput?.plainText,
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
    getSelectedIndex: (interactionId) => interactionChoiceSelection.get(interactionId),
    setSelectedIndex: (interactionId, index) => {
      interactionChoiceSelection.set(interactionId, index)
    },
    getCustomReply: (interactionId) => interactionCustomReplies.get(interactionId) ?? "",
    setCustomReply: (interactionId, reply) => {
      interactionCustomReplies.set(interactionId, reply)
    },
    clearCustomReply: (interactionId) => {
      interactionCustomReplies.delete(interactionId)
    },
    isCustomEditing: (interactionId) => interactionCustomEditing.has(interactionId),
    setCustomEditing: (interactionId, editing) => {
      if (editing) {
        interactionCustomEditing.add(interactionId)
      } else {
        interactionCustomEditing.delete(interactionId)
      }
    },
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

  const handleSigint = () => {
    void (activePrompt() ? requestPromptStop() : requestExit())
  }
  const promptKeyDownController = createPromptKeyDownController({
    handleFocusedInteractionKey,
    handleCommandCenterKey,
    isAttached,
    promptFocused: () => Boolean(promptInput?.focused),
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
    getPromptText: () => promptInput ? promptTextController.currentText() : undefined,
    getPromptOffsets: () => visibleTranscriptEntries()
      .filter((entry) => entry.role === "user")
      .map((entry) => transcriptRenderables.get(entry.id)?.wrapper.y ?? null)
      .filter((offset): offset is number => offset !== null),
    getScrollState: () => transcriptScrollbox
      ? { left: transcriptScrollbox.scrollLeft, top: transcriptScrollbox.scrollTop }
      : null,
    scrollTo: (position) => {
      transcriptScrollbox?.scrollTo(position)
    },
    requestRender: () => {
      transcriptScrollbox?.requestRender()
    },
    setLastTranscriptScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
  })
  const waitingRoomKeyController = createWaitingRoomKeyController({
    isAttached,
    hotkeysOpen: dialogOverlayOpen,
    promptFocused: () => Boolean(promptInput?.focused),
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
    promptFocused: () => Boolean(promptInput?.focused),
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

  const automationSnapshot = () => {
    return buildCliAutomationSnapshot({
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
      interactionChoiceSelection: (interactionId) => interactionChoiceSelection.get(interactionId) ?? 0,
      interactionCustomReply: (interactionId) => interactionCustomReplies.get(interactionId) ?? "",
      interactionCustomEditing: (interactionId) => interactionCustomEditing.has(interactionId),
    })
  }

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
  automationServerController.start()
  process.on("SIGINT", handleSigint)
  process.stdin.on("data", handleStdinData)
  onCleanup(() => {
    process.off("SIGINT", handleSigint)
    process.stdin.off("data", handleStdinData)
    automationServerController.stop()
    terminalOutputRecordQueue.clearTimer()
  })

  const onResize = () => {
    if (isAttached()) {
      void maybeResize(client, sessionState().id)
    }
  }

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
    isClosing: () => closing,
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

  // Track daemon activity for connection health monitoring
  const recordDaemonActivity = (activityType: string) => {
    connectionHealthWatchdogController.recordActivity()
    // If we were showing a stale connection warning, clear it
    if (daemonDisconnected()) {
      setDaemonDisconnected(false)
      updateSessionChrome()
    }
  }

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

  const applyKernelSessionSnapshot = async (
    nextSession: RuntimeSession,
    nextProviderRun: RuntimeProviderRun | null,
  ) => {
    const previousSession = sessionState()
    const projectedSession = applyProviderRunProfileToSession(nextSession, nextProviderRun ?? providerRunState())
    const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(projectedSession)
    const promptJustCompleted = sessionHasPromptWork(previousSession) && !sessionHasPromptWork(projectedSession)

    applySessionState(projectedSession)

    const activeRun = providerRunState()
    if (nextProviderRun) {
      if (!activeRun || !sameProviderRun(activeRun, nextProviderRun)) {
        logProviderRunDebug("kernel event refreshed provider run", nextProviderRun, {
          session_id: nextSession.id,
          previous_provider_run_id: activeRun?.id ?? null,
        })
        setProviderRunState(nextProviderRun)
        updateSessionChrome()
      }
    } else if (activeRun) {
      logProviderRunDebug("kernel event cleared provider run", activeRun, {
        session_id: nextSession.id,
      })
      setProviderRunState(null)
      updateSessionChrome()
      if (!supportsKernelEventStream && sessionHasPromptWork(projectedSession)) {
        void recoverProviderRun("missing active provider run")
      }
    }

    if (shouldRefreshPanes || promptJustCompleted) {
      await refreshAgentPanes(projectedSession)
    }
  }

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

  const handleKernelEvent = async (event: KernelEvent) => {
    switch (event.event) {
      case "terminal_output":
        recordDaemonActivity("kernel_terminal_output")
        queueTerminalOutputRecords(event.records as TerminalOutputRecord[])
        return
      case "runtime_notices":
        kernelEventController.applyRuntimeNotices(event.notices as RuntimeNoticeRecord[])
        return
      case "assistant_message_completed":
        kernelEventController.applyAssistantMessageCompleted(event)
        return
      case "session_snapshot":
        recordDaemonActivity("kernel_session_snapshot")
        scheduleSharedPromptInputHistoryRefresh()
        await applyKernelSessionSnapshot(
          normalizeRuntimeSession({
            ...(event.session as RuntimeSession),
            ...((event.agent_activity && typeof event.agent_activity === "object")
              ? { agent_activity: event.agent_activity }
              : {}),
          } as RuntimeSession),
          (event.provider_run as RuntimeProviderRun | null) ?? null,
        )
        return
      case "heartbeat":
        recordDaemonActivity("kernel_heartbeat")
        scheduleSharedPromptInputHistoryRefresh()
        return
      case "session_unavailable":
        await handleKernelSessionUnavailable(event.message)
        return
      case "relay_status_changed":
        void refreshWaitingRoomData()
        return
      case "remote_machines_changed":
        void refreshWaitingRoomData()
        return
      case "waiting_room_inventory_changed":
        void refreshWaitingRoomData()
        return
      case "transport_resumed":
        kernelEventController.applyTransportResumed()
        scheduleSharedPromptInputHistoryRefresh()
        void resyncAttachedKernelState("transport_resumed")
        return
      case "replay_gap":
        recordDaemonActivity("kernel_replay_gap")
        appendNotice("Missed retained kernel events, refreshed session state.", "warning")
        flashFooter("Missed retained kernel events, refreshed session state.", "info")
        void resyncAttachedKernelState("replay_gap")
        return
      case "transport_closed":
        kernelEventController.applyTransportClosed(event.message)
        void recoverAttachedSessionAfterKernelRestart()
        return
    }
  }

  const handleKernelSessionUnavailable = async (message: string) => {
    const sessionId = sessionState().id
    if (isAttached() && sessionId) {
      try {
        const nextSession = await getSessionState(client, sessionId)
        const nextAttachment = await attachToSession(client, sessionId, options.clientId)
        if (!isAttached() || sessionState().id !== sessionId) {
          return
        }
        setAttachmentState(nextAttachment)
        applySessionState(applyProviderRunProfileToSession(nextSession, providerRunState()))
        kernelEventSubscriptionController.reset()
        await syncKernelEventSubscription()
        await refreshAgentPanes(sessionState())
        clearLocalBusyStateForAuthoritativeIdle(sessionState())
        recordDaemonActivity("kernel_session_unavailable_recovered")
        setDaemonDisconnected(false)
        setStatusLine(DEFAULT_CONNECTED_STATUS)
        updateSessionChrome()
        return
      } catch (error) {
        appLogger?.debug("session unavailable confirmed by state lookup failure", {
          session_id: sessionId,
          message,
          error: formatError(error),
        })
      }
    }
    await transitionToNoSession(message)
  }

  const startConnectionWatchdog = connectionHealthWatchdogController.start

  const pollOutput = async () => {
    await runPollingLoop({
      operation: "polling terminal output",
      intervalMs: 50,
      isClosing: () => closing,
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
      task: async () => {
      const attachment = attachmentState()
      if (!attachment) {
        return
      }
      let records: TerminalOutputRecord[]
      try {
        records = await pumpTerminalOutput(client, sessionState().id, attachment.id)
      } catch (error) {
        const message = formatError(error)
        if (/has no active provider run/i.test(message) && !sessionHasPromptWork(sessionState())) {
          setProviderRunState(null)
          updateSessionChrome()
          return
        }
        throw error
      }
      if (records.length > 0) {
        recordDaemonActivity("terminal_output")
      }
      queueTerminalOutputRecords(records)
      },
      sleep,
    })
  }

  const pollNotices = async () => {
    await runPollingLoop({
      operation: "polling runtime notices",
      intervalMs: 150,
      isClosing: () => closing,
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
      task: async () => {
      const attachment = attachmentState()
      if (!attachment) {
        return
      }
      const notices = await pollRuntimeNotices(client, sessionState().id, attachment.id)
      recordDaemonActivity("runtime_notices")
      for (const notice of notices) {
        appendNotice(notice.message)
      }
      },
      sleep,
    })
  }

  const pollSessionState = async () => {
    await runPollingLoop({
      operation: "polling session state",
      intervalMs: 250,
      isClosing: () => closing,
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
      task: async () => {
      if (!isAttached()) {
        return
      }
      const previousSession = sessionState()
      const session = await getSessionState(client, sessionState().id)
      recordDaemonActivity("session_state_poll")
      const projectedSession = applyProviderRunProfileToSession(session, providerRunState())
      const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(projectedSession)
      const promptJustCompleted = sessionHasPromptWork(previousSession) && !sessionHasPromptWork(projectedSession)
      applySessionState(projectedSession)
      if (shouldRefreshPanes || promptJustCompleted) {
        await refreshAgentPanes(projectedSession)
      }
      if (session.active_provider_run_id) {
        const activeRun = providerRunState()
        const run = await tryGetProviderRun(client, session.active_provider_run_id, appLogger)
        if (run && (!activeRun || !sameProviderRun(activeRun, run))) {
          logProviderRunDebug("session poll refreshed provider run", run, {
            session_id: session.id,
            previous_provider_run_id: activeRun?.id ?? null,
            previous_model: activeRun?.model ?? null,
            previous_variant: activeRun?.variant ?? null,
            previous_usage_tokens_total: activeRun?.usage_tokens_total ?? null,
            refresh_reason: !activeRun
              ? "missing_run"
              : activeRun.id !== session.active_provider_run_id
                ? "run_changed"
                : activeRun.usage_tokens_total !== run.usage_tokens_total
                  ? "usage_changed"
                  : activeRun.model !== run.model
                    ? "model_changed"
                    : activeRun.variant !== run.variant
                      ? "variant_changed"
                      : activeRun.state !== run.state
                        ? "state_changed"
                        : "metadata_changed",
          })
          setProviderRunState(run)
          applySessionState(applyProviderRunProfileToSession(sessionState(), run))
          updateSessionChrome()
        }
      } else if (providerRunState()) {
        logProviderRunDebug("session poll cleared provider run", providerRunState(), {
          session_id: session.id,
        })
        setProviderRunState(null)
        updateSessionChrome()
        if (sessionHasPromptWork(session)) {
          void recoverProviderRun("missing active provider run")
        }
      }
      },
      sleep,
    })
  }

  const backgroundPollerStartupController = createBackgroundPollerStartupController({
    logger: appLogger,
    ready: () => Boolean(promptInput && transcriptScrollbox),
    promptMounted: () => Boolean(promptInput),
    transcriptScrollTop: () => transcriptScrollbox?.scrollTop ?? 0,
    setLastTranscriptScrollTop: (scrollTop) => {
      lastTranscriptScrollTop = scrollTop
    },
    isAttached,
    rebuildTranscript,
    syncPromptPlaceholder,
    focusPrompt: () => {
      promptInput?.focus()
    },
    blurPrompt: () => {
      promptInput?.blur()
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
    closing = true
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
        responseLayoutBox = value
        logViewDebug("mounted response layout box")
        applyResponseLayout()
      }}
      onResponseRowBoxRef={(index, value) => {
        responseRowBoxes[index] = value
        applyResponseLayout()
      }}
      onPaneGridBorderRowRef={(index, value) => {
        paneGridBorderRows[index] = value
        applyResponseLayout()
      }}
      onPaneGridBottomBorderRowRef={(value) => {
        paneGridBottomBorderRow = value
        applyResponseLayout()
      }}
      onPaneGridHorizontalSegmentRef={(rowIndex, segmentIndex, value) => {
        paneGridHorizontalSegments[rowIndex] ??= []
        paneGridHorizontalSegments[rowIndex][segmentIndex] = value
        applyResponseLayout()
      }}
      onPaneGridBottomHorizontalSegmentRef={(segmentIndex, value) => {
        paneGridBottomHorizontalSegments[segmentIndex] = value
        applyResponseLayout()
      }}
      onPaneGridJunctionTextRef={(rowIndex, junctionIndex, value) => {
        paneGridJunctionTexts[rowIndex] ??= []
        paneGridJunctionTexts[rowIndex][junctionIndex] = value
        applyResponseLayout()
      }}
      onPaneGridBottomJunctionTextRef={(junctionIndex, value) => {
        paneGridBottomJunctionTexts[junctionIndex] = value
        applyResponseLayout()
      }}
      onPaneGridVerticalSegmentRef={(rowIndex, segmentIndex, value) => {
        paneGridVerticalSegments[rowIndex] ??= []
        paneGridVerticalSegments[rowIndex][segmentIndex] = value
        applyResponseLayout()
      }}
      onResponsePrimaryPaneRef={(value) => {
        responsePrimaryPane = value
        logViewDebug("mounted response primary pane")
        applyResponseLayout()
      }}
      onHistoryLoadingBoxRef={(value) => {
        historyLoadingBox = value
        logViewDebug("mounted history loading box")
        renderHistoryLoadingIndicator()
      }}
      onTranscriptScrollboxRef={(value) => {
        transcriptScrollbox = value
        logViewDebug("mounted primary transcript scrollbox")
        rebuildTranscript()
        ensureBackgroundPollersStarted()
      }}
      onResponsePrimaryInteractionBoxRef={(value) => {
        responsePrimaryInteractionBox = value
        renderAgentInteractions()
        applyResponseLayout()
      }}
      onResponsePrimaryFooterBoxRef={(value) => {
        responsePrimaryFooterBox = value
        renderSplitPaneFooters()
        applyResponseLayout()
      }}
      onResponseAuxiliaryPaneRef={(index, value) => {
        responseAuxiliaryPanes[index] = value
        logViewDebug("mounted response auxiliary pane", {
          pane_index: index + 1,
        })
        applyResponseLayout()
      }}
      onResponseAuxiliaryScrollboxRef={(index, value) => {
        responseAuxiliaryScrollboxes[index] = value
        logViewDebug("mounted response auxiliary scrollbox", {
          pane_index: index + 1,
        })
        applyResponseLayout()
      }}
      onResponseAuxiliaryInteractionBoxRef={(index, value) => {
        responseAuxiliaryInteractionBoxes[index] = value
        renderAgentInteractions()
        applyResponseLayout()
      }}
      onResponseAuxiliaryFooterBoxRef={(index, value) => {
        responseAuxiliaryFooterBoxes[index] = value
        renderSplitPaneFooters()
        applyResponseLayout()
      }}
      onCommandCenterBoxRef={(value) => {
        commandCenterBox = value
        renderCommandCenter()
      }}
      onPromptInputRef={(value) => {
        promptInput = value
        value.syntaxStyle = promptAttachmentTokenStyle
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
      onPromptMetaProviderTextRef={(value) => {
        promptMetaProviderText = value
        updateSessionChrome()
      }}
      onPromptMetaProviderDividerTextRef={(value) => {
        promptMetaProviderDividerText = value
        updateSessionChrome()
      }}
      onPromptMetaModelTextRef={(value) => {
        promptMetaModelText = value
        updateSessionChrome()
      }}
      onPromptMetaModelDividerTextRef={(value) => {
        promptMetaModelDividerText = value
        updateSessionChrome()
      }}
      onPromptMetaVariantTextRef={(value) => {
        promptMetaVariantText = value
        updateSessionChrome()
      }}
      onPromptMetaUsageDividerTextRef={(value) => {
        promptMetaUsageDividerText = value
        updateSessionChrome()
      }}
      onPromptMetaUsageTokensTextRef={(value) => {
        promptMetaUsageTokensText = value
        updateSessionChrome()
      }}
      onPromptMetaUsageBarOpenTextRef={(value) => {
        promptMetaUsageBarOpenText = value
        updateSessionChrome()
      }}
      onPromptMetaUsageBarFilledTextRef={(value) => {
        promptMetaUsageBarFilledText = value
        updateSessionChrome()
      }}
      onPromptMetaUsageBarEmptyTextRef={(value) => {
        promptMetaUsageBarEmptyText = value
        updateSessionChrome()
      }}
      onPromptMetaUsageBarCloseTextRef={(value) => {
        promptMetaUsageBarCloseText = value
        updateSessionChrome()
      }}
      onPromptMetaUsagePercentTextRef={(value) => {
        promptMetaUsagePercentText = value
        updateSessionChrome()
      }}
      onStatusIndicatorBoxRef={(value) => {
        statusIndicatorBox = value
        updateSessionChrome()
      }}
      onFooterSummaryBoxRef={(value) => {
        footerSummaryBox = value
        updateSessionChrome()
      }}
      onHotkeysOverlayBoxRef={(value) => {
        hotkeysOverlayBox = value
        renderHotkeysOverlay()
      }}
    />
  )
}

function formatError(error: unknown): string {
  return describeCliError(error)
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
