import process from "node:process"
import { randomBytes } from "node:crypto"
import { homedir } from "node:os"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, MouseButton, RGBA, ScrollBoxRenderable, TextAttributes, TextRenderable, addDefaultParsers, parseKeypress, type KeyBinding, type Renderable, type TextareaRenderable } from "@opentui/core"
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
  type CliAutomationServer,
} from "./cli-automation.js"
import { createCliAutomationActionHandler } from "./cli-automation-handler.js"
import { buildCliAutomationSnapshot } from "./cli-automation-snapshot.js"
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
import {
  describeCliDialogFocusTarget,
  type CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"
import { createCliDialogOverlayController } from "./cli-dialog-overlay-controller.js"
import {
  renderCliDialogOverlay,
} from "./cli-dialog-overlay.js"
import {
  computeTranscriptRebuildScrollTop,
  nextWaitingRoomIntroStep,
} from "./background-effects.js"
import {
  executeSlashCommand,
} from "./commands.js"
import { createCommandCenterController } from "./command-center-controller.js"
import { renderCommandCenterOverlay } from "./command-center-renderer.js"
import { refreshAgentPaneState, selectCurrentAgentPaneEntries, trimAgentPaneEntries } from "./agent-pane-state.js"
import { parseProviderNamespaceCommand } from "./provider-command-catalog.js"
import { validateProviderNamespaceSubmit } from "./provider-namespace-submit-policy.js"
import { createClipboardController } from "./clipboard-controller.js"
import {
  createFooterFlashController,
  type FooterFlash,
} from "./footer-flash-controller.js"
import { HOTKEY_TOGGLE_LABEL, matchHotkeysToggleEvent, shouldCycleFocusOnTabEvent, shouldHandleWaitingRoomKeyEvent } from "./hotkeys.js"
import { buildHotkeySections } from "./hotkey-help.js"
import { createHistoryScrollRestoreController } from "./history-scroll-restore-controller.js"
import { clampScrollTop, findTurnPromptScrollTarget, promptTurnNavigationDirectionForKey } from "./history-viewport.js"
import { renderHistoryLoadingIndicator as renderHistoryLoadingIndicatorView } from "./history-loading-renderer.js"
import { createDefaultShellContext, type ShellContext } from "@arroba/kernel-client/shell-core"
import { KernelEvent, LocalIpcClient } from "./ipc.js"
import {
  appendInteractionCustomReply,
  deleteInteractionCustomReply,
  interactionCustomChoiceIndex,
  nextInteractionChoiceIndex,
  resolveInteractionChoiceKeyAction,
  resolveInteractionChoiceSubmission,
} from "./interaction-choice-state.js"
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
import {
  createPromptHistoryNavigationController,
} from "./prompt-history-navigation-controller.js"
import {
  createPromptInputHistoryRefreshController,
} from "./prompt-input-history-refresh-controller.js"
import { createPromptInputHistoryController } from "./prompt-input-history-controller.js"
import {
  createPromptSubmissionUiController,
  type SubmittedPromptUiSnapshot,
} from "./prompt-submission-ui-controller.js"
import {
  createPromptTextController,
} from "./prompt-text-controller.js"
import { createPromptStopController } from "./prompt-stop-controller.js"
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
  formatPromptSubmissionBody,
  formatPromptSubmissionStatusLine,
  pendingPromptAttachmentsToParts,
} from "./prompt-submission-state.js"
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
  applyHistoryDeferral,
  hydrateTranscriptEntries,
  markDeferredHistoryEntries,
} from "./transcript-history.js"
import {
  createPromptContentChangeController,
} from "./prompt-content-change-controller.js"
import { createPromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { buildPaneGridModel, type PaneGridTone } from "./response-pane-grid.js"
import {
  responsePaneRowSlots,
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "./response-panes.js"
import {
  extractPromptHistoryEntries,
  resolvePromptHistoryKeyNavigation,
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
  getToolActivityLabel,
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
  derivePromptLifecycleTransition,
  buildDetachedSessionState,
  deriveSessionTransitionState,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionResponseLayout,
  shouldConfirmIdleTurnCompletion,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "./session-state.js"
import { createSessionAttachmentController } from "./session-attachment-controller.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createTranscriptHistoryLoadController } from "./transcript-history-load-controller.js"
import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
} from "./transcript-display.js"
import {
  reindexTranscriptEntries,
  trimSingleTrailingNewline,
} from "./transcript-text.js"
import { resolveTerminalRecordAgentId as resolveTerminalRecordAgentIdFromState } from "./terminal-record-agent-resolver.js"
import { createTranscriptHistoryAutoloadController } from "./transcript-history-autoload-controller.js"
import {
  createTerminalOutputRecordQueue,
} from "./terminal-output-record-queue.js"
import { createTerminalExitController } from "./terminal-exit-controller.js"
import { createTranscriptRenderDeferralController } from "./transcript-render-deferral-controller.js"
import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  shouldSkipConsecutiveTranscriptEntry,
  shouldRenderProviderStatus,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import {
  decideBootstrapAction,
  SESSION_NEW_ERROR_HINT,
  SESSION_NEW_FOOTER_HINT,
  SESSION_NEW_PLACEHOLDER,
  formatSessionDisplayLabel,
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
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomControlActivationDecision,
  deriveWaitingRoomDeleteDecision,
  deriveWaitingRoomKeyNavigationDecision,
  deriveWaitingRoomSessionLifecycleDecision,
  deriveWaitingRoomStateUpdate,
  waitingRoomSessionLifecycleActionForEvent,
  type WaitingRoomSessionLifecycleAction,
} from "./waiting-room-controller.js"
import {
  getWaitingRoomInventory,
  type RemoteKernelView,
  type RemoteMachineView,
} from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import {
  createWaitingRoomState,
  type WaitingRoomFocus,
  type WaitingRoomState,
} from "./waiting-room.js"
import { createWaitingRoomTransitionController } from "./waiting-room-transition-controller.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import {
  primeWaitingRoomWorktreeInventory,
} from "./waiting-room-worktrees.js"
import {
  resolveWorkspaceVisibleAgents,
  resolveWorkspaceVisibleTranscriptAgentId,
  type WorkspaceScreenMode,
} from "./workspace-screen.js"
import {
  isWorkspaceShellCommand,
  type WorkspaceShellEntry,
} from "./workspace-shell.js"
import { submitWorkspaceShellCommand as submitWorkspaceShellCommandWithDeps } from "./workspace-shell-controller.js"
import { createWorkflowController, deriveWorkflowSelectionState } from "./workflow-controller.js"
import {
  deriveWorkflowPromptState,
  formatWorkflowPromptPlaceholder,
  isWorkflowCommandInput,
  resolveActiveWorkflowRun,
} from "./workflow-prompt-state.js"
import { createWorkflowPromptSubmitController } from "./workflow-prompt-submit-controller.js"
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
  appendPreviewLine,
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
import { reconcileMountedTranscriptPane } from "./transcript-pane-reconcile.js"
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
  const configuredProviderPreferences = preferences.providers?.[options.provider ?? "opencode"]
  if (options.model === "default") {
    options.model = configuredProviderPreferences?.model ?? options.model
  }
  if (!options.effort.trim()) {
    options.effort = configuredProviderPreferences?.effort ?? options.effort
  }
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

  const agentLocationLabel = (agent: AgentInstance | null | undefined): string | null => {
    const remote = agent?.remote_execution
    if (!remote) return null
    const slice = slicesState().find((candidate) =>
      candidate.worker_kernel_id === remote.worker_kernel_id
      || candidate.worker_kernel_ref === remote.worker_kernel_id
      || candidate.worker_machine_id === remote.worker_machine_id
    )
    if (slice) {
      return `slice:${slice.name || slice.id}`
    }
    return `remote:${remote.worker_kernel_id}`
  }
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
  const [workflowInspectorMode, setWorkflowInspectorMode] = createSignal<"runtime" | "terminal">("runtime")
  type WorkflowNodeInstructionsEditor = {
    workflowId: string
    nodeId: string
    draft: string
  }
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
  let workflowNodeInstructionsInput: TextareaRenderable | undefined
  let hotkeysOverlayBox: BoxRenderable | undefined
  const sessionChromeSummaryRenderState = createSessionChromeSummaryRenderState()
  let historyLoadingText: TextRenderable | undefined
  const statusIndicatorRenderState = createStatusIndicatorRenderState()
  let closing = false
  const degradedPollers = new Set<string>()
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
  let hydratedPromptHistorySessionId: string | null | undefined
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
    setWorkspaceShellContext((previous) => ({
      ...previous,
      workspace: session.workspace_id || previous.workspace,
      worktree: session.worktree_id || previous.worktree,
      sessionId: session.id,
      attachmentId: attachmentState()?.id ?? previous.attachmentId,
      agentId: session.focused_agent_id ?? session.agents[0]?.id ?? previous.agentId,
    }))
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

  createEffect(() => {
    const nextSelection = deriveWorkflowSelectionState(
      sessionState().workflows ?? [],
      selectedWorkflowId(),
      selectedWorkflowNodeId(),
    )
    const nextWorkflowId = nextSelection.workflowId
    if (selectedWorkflowId() !== nextWorkflowId) {
      setSelectedWorkflowId(nextWorkflowId)
    }
    const nextNodeId = nextSelection.nodeId
    if (selectedWorkflowNodeId() !== nextNodeId) {
      setSelectedWorkflowNodeId(nextNodeId)
    }
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
  const formatAgentLabel = (agent: AgentInstance | null | undefined) => {
    if (!agent) {
      return ""
    }
    return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  }
  const resolveSessionAgent = (reference?: string | null) => {
    const normalizedReference = reference?.trim() ?? ""
    if (!normalizedReference) {
      const agent = focusedAgent()
      return agent
        ? { agent, error: null }
        : { agent: null, error: "no focused agent available" }
    }

    const matches = sessionState().agents.filter((agent) => {
      return agent.id === normalizedReference
        || agent.agent_ref === normalizedReference
        || agent.alias === normalizedReference
    })
    if (matches.length === 1) {
      return { agent: matches[0], error: null }
    }
    if (matches.length > 1) {
      return { agent: null, error: `multiple agents match '${normalizedReference}'` }
    }
    return { agent: null, error: `agent '${normalizedReference}' not found` }
  }
  const workflowNodeInstructionsInspector = () => {
    const editor = workflowNodeInstructionsEditor()
    if (!editor) {
      return null
    }
    const workflow = sessionState().workflows?.find((entry) => entry.id === editor.workflowId) ?? null
    const node = workflow?.nodes?.find((entry) => entry.id === editor.nodeId) ?? null
    const agent = node ? sessionState().agents.find((entry) => entry.id === node.agent_id) ?? null : null
    const workflowLabel = workflow?.alias ? `${workflow.id} (${workflow.alias})` : editor.workflowId
    const agentLabel = agent ? formatAgentLabel(agent) : node?.agent_id ?? "unknown"
    return {
      title: "Node Instructions",
      meta: [
        `Workflow: ${workflowLabel}`,
        `Node: ${node?.id ?? editor.nodeId}`,
        `Agent: ${agentLabel}`,
      ],
      draft: editor.draft ?? "",
      placeholder: "Type system instructions for this node",
      hint: "Use /workflow node instructions save to persist. /workflow node instructions close to discard.",
      onDraftChange: (draft: string) => updateWorkflowNodeInstructionsDraft(draft),
      onEditorRef: (editorRef: TextareaRenderable | null) => {
        workflowNodeInstructionsInput = editorRef ?? undefined
      },
    }
  }
  const workflowRuntimeInspector = () => {
    const workflow = sessionState().workflows?.find((entry) => entry.id === selectedWorkflowId()) ?? null
    if (!workflow) {
      return null
    }
    const selectedNodeId = selectedWorkflowNodeId()
    const selectedNode = workflow.nodes?.find((entry) => entry.id === selectedNodeId) ?? null
    const selectedAgent = selectedNode
      ? sessionState().agents.find((entry) => entry.id === selectedNode.agent_id) ?? null
      : null
    const workflowRun = resolveActiveWorkflowRun(workflow.id, sessionState().workflow_runs ?? [])
      ?? [...(sessionState().workflow_runs ?? [])]
        .filter((entry) => entry.workflow_id === workflow.id)
        .sort((left, right) => right.created_at_ms - left.created_at_ms)[0]
      ?? null
    const workflowLabel = workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
    const meta = [
      `Workflow: ${workflowLabel}`,
      `Selected node: ${selectedNode?.id ?? "-"}`,
      `Agent: ${selectedAgent ? formatAgentLabel(selectedAgent) : selectedNode?.agent_id ?? "-"}`,
      `Run: ${workflowRun?.id ?? "-"}`,
      `Run status: ${String(workflowRun?.status ?? "idle").toLowerCase()}`,
    ]
    const nodeRuns = workflowRun?.node_runs ?? []
    const selectedNodeRun = selectedNode
      ? [...nodeRuns].filter((entry) => entry.node_id === selectedNode.id).sort((left, right) => right.created_at_ms - left.created_at_ms)[0] ?? null
      : null
    const failureEvents = workflowRun?.failure_events ?? []
    const selectedNodeFailures = selectedNodeRun
      ? failureEvents.filter((entry) => entry.source_node_run_id === selectedNodeRun.id)
      : []
    const workflowWatchdogs = (sessionState().workflow_watchdogs ?? [])
      .filter((entry) => entry.workflow_id === workflow.id)
      .sort((left, right) => left.next_run_at_ms - right.next_run_at_ms)
    const lines: string[] = []
    lines.push(`Watchdogs: ${workflowWatchdogs.length}`)
    if (workflowWatchdogs.length > 0) {
      lines.push("")
      lines.push("Watchdogs")
      for (const watchdog of workflowWatchdogs.slice(0, 8)) {
        lines.push(`- ${watchdog.id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)}`)
        lines.push(`  next: ${new Date(watchdog.next_run_at_ms).toISOString()}`)
        if (watchdog.last_status) {
          lines.push(`  last: ${watchdog.last_status}`)
        }
        if (watchdog.pending_run) {
          lines.push("  pending: true")
        }
      }
    }
    lines.push("")
    lines.push(`Failure events: ${failureEvents.length}`)
    if (selectedNodeRun) {
      lines.push("")
      lines.push("Selected node run")
      lines.push(`- id: ${selectedNodeRun.id}`)
      lines.push(`- status: ${String(selectedNodeRun.status).toLowerCase()}`)
      lines.push(`- summary: ${selectedNodeRun.summary ?? "-"}`)
      if (selectedNodeRun.turn_envelope) {
        lines.push(`- turn state: ${selectedNodeRun.turn_envelope.state}`)
        lines.push(`- delivery token: ${selectedNodeRun.turn_envelope.delivery_token}`)
        if (selectedNodeRun.turn_envelope.mailbox_content) {
          lines.push("")
          lines.push("Mailbox snapshot")
          lines.push(selectedNodeRun.turn_envelope.mailbox_content)
        }
        if (selectedNodeRun.turn_envelope.handoff_payloads_json) {
          lines.push("")
          lines.push("Handoff snapshot")
          lines.push(selectedNodeRun.turn_envelope.handoff_payloads_json)
        }
        const runtimeToolCalls = selectedNodeRun.turn_envelope.runtime_tool_calls ?? []
        if (runtimeToolCalls.length > 0) {
          lines.push("")
          lines.push("Runtime tool calls")
          for (const call of runtimeToolCalls.slice(-10)) {
            lines.push(`- ${call.tool_name} @ ${new Date(call.timestamp_ms).toISOString()} ok=${String(call.ok)}`)
            lines.push(`  args: ${call.arguments_json}`)
            if (call.result_json) {
              lines.push(`  result: ${call.result_json}`)
            }
          }
        }
      }
    }
    if (selectedNodeFailures.length > 0) {
      lines.push("")
      lines.push("Selected node failure events")
      for (const failure of selectedNodeFailures) {
        lines.push(`- ${String(failure.kind).toLowerCase()} @ ${new Date(failure.timestamp_ms).toISOString()}`)
        lines.push(`  ${failure.message}`)
        if (failure.edge_ids.length > 0) {
          lines.push(`  edges: ${failure.edge_ids.join(", ")}`)
        }
      }
    } else if (failureEvents.length > 0) {
      lines.push("")
      lines.push("Recent workflow failure events")
      for (const failure of failureEvents.slice(-5).reverse()) {
        lines.push(`- ${String(failure.kind).toLowerCase()} @ ${new Date(failure.timestamp_ms).toISOString()}`)
        lines.push(`  ${failure.message}`)
      }
    } else {
      lines.push("")
      lines.push("No failure events recorded for the current workflow run.")
    }
    return {
      title: "Workflow Runtime",
      meta,
      body: lines.join("\n"),
      hint: "Use /workflow runs, /workflow cancel, and /workflow resume to manage the current run.",
    }
  }
  const workflowTerminalInspector = () => {
    const workflow = sessionState().workflows?.find((entry) => entry.id === selectedWorkflowId()) ?? null
    if (!workflow) {
      return null
    }
    const workflowLabel = workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
    const consoleState = (sessionState().workflow_consoles ?? []).find((entry) => entry.workflow_id === workflow.id) ?? null
    const body = (consoleState?.entries ?? []).map((entry) => entry.text ?? "").join("")
    return {
      title: "Workflow Terminal",
      meta: [
        `Workflow: ${workflowLabel}`,
        `Entries: ${consoleState?.entries?.length ?? 0}`,
      ],
      body: body.length > 0 ? body : "No workflow terminal output yet.",
      hint: "Use /workflow terminal [workflow-ref] to keep this console visible while the workflow runs.",
    }
  }
  const workflowInspector = () => workflowNodeInstructionsEditor()
    ? workflowNodeInstructionsInspector()
    : workflowInspectorMode() === "terminal"
      ? workflowTerminalInspector()
      : workflowRuntimeInspector()
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
  const inspectHotkeysToggleShortcut = (
    source: "keyboard" | "stdin" | "textarea",
    event: { name: string; ctrl?: boolean; meta?: boolean; super?: boolean; eventType?: string; baseCode?: number },
  ) => {
    const match = matchHotkeysToggleEvent(event)
    if (match.normalizedName === "t" || event.ctrl || event.meta || event.super) {
      appLogger?.debug("evaluated hotkeys toggle shortcut", {
        source,
        matched: match.matched,
        reason: match.reason,
        key_name: event.name,
        normalized_name: match.normalizedName,
        event_type: event.eventType ?? null,
        ctrl: Boolean(event.ctrl),
        meta: Boolean(event.meta),
        super: Boolean(event.super),
        base_code: event.baseCode ?? null,
        hotkeys_open: hotkeysOpen(),
      })
    }
    return match
  }
  const handleHotkeysToggleShortcut = (
    source: "keyboard" | "stdin" | "textarea",
    event: {
      name: string
      ctrl?: boolean
      meta?: boolean
      super?: boolean
      eventType?: string
      baseCode?: number
      defaultPrevented?: boolean
      preventDefault?: () => void
      stopPropagation?: () => void
    },
  ) => {
    if (event.defaultPrevented) {
      return false
    }
    const hotkeysToggle = inspectHotkeysToggleShortcut(source, event)
    if (!hotkeysToggle.matched) {
      return false
    }
    event.preventDefault?.()
    event.stopPropagation?.()
    const previousHotkeysOpen = hotkeysOpen()
    hotkeyDebug(`shortcut ${source} matched reason=${hotkeysToggle.reason} open=${previousHotkeysOpen} key=${event.name}`)
    appLogger?.debug("toggling hotkeys via shortcut", {
      source,
      reason: hotkeysToggle.reason,
      hotkeys_open: previousHotkeysOpen,
      next_hotkeys_open: !previousHotkeysOpen,
      current_focus: describeRenderableDebug(currentFocusedRenderable()),
    })
    toggleHotkeys()
    hotkeyDebug(`shortcut ${source} finished open=${hotkeysOpen()} saved=${dialogOverlayController.savedFocusDebug()?.type ?? "none"}`)
    appLogger?.debug("finished toggling hotkeys via shortcut", {
      source,
      reason: hotkeysToggle.reason,
      previous_hotkeys_open: previousHotkeysOpen,
      hotkeys_open: hotkeysOpen(),
      saved_focus: dialogOverlayController.savedFocusDebug(),
      current_focus: describeRenderableDebug(currentFocusedRenderable()),
    })
    return true
  }
  const reconcileWaitingRoom = (next: WaitingRoomState) => {
    const currentState = waitingRoomState()
    const update = deriveWaitingRoomStateUpdate({
      currentState,
      nextState: next,
      sessions: availableSessions(),
      catalog: providerCatalogState(),
      remote: {
        cloudNotice: waitingRoomCloudNotice(),
        inventoryStatus: waitingRoomInventoryStatus(),
        loadingFrame: waitingRoomState().introStep,
        relay: relayStatusState(),
        machines: remoteMachinesState(),
        kernels: remoteKernelsState(),
        terminals: terminalsState(),
        slices: slicesState(),
      },
      themeRegistry: themeRegistryState(),
      currentProvider: (options.provider ?? "opencode") as BackendProviderId,
      currentModel: options.model,
    })
    setWaitingRoomState(update.normalizedState)
    options.provider = update.nextProvider
    options.model = update.nextModel
    options.effort = update.nextEffort
    if (currentState.themeId !== update.normalizedState.themeId) {
      const nextThemeId = applyTheme(update.normalizedState.themeId, themeRegistryState())
      transcriptSyntax = createTranscriptSyntaxStyle()
      setThemeRevision((revision) => revision + 1)
      void saveUiPreferences({ theme: nextThemeId })
      setPreferencesState((current) => mergeUiPreferences(current, { theme: nextThemeId }))
      applyResponseLayout()
      renderCommandCenter()
    }
    if (update.shouldPersistProviderPreferences) {
      void saveProviderPreferences(update.nextProvider, {
        model: options.model,
        effort: options.effort,
      })
    }
    if (!isAttached()) {
      rebuildTranscript()
    }
    updateSessionChrome()
    syncCommandCenter()
    return update.normalizedState
  }
  const activateWaitingRoom = async () => {
    try {
      if (!kernelConnected()) {
        await connectDetachedKernelFromWaitingRoom()
      }
      const remote = {
        relay: relayStatusState(),
        machines: remoteMachinesState(),
        kernels: remoteKernelsState(),
        terminals: terminalsState(),
        slices: slicesState(),
      }
      const controlDecision = deriveWaitingRoomControlActivationDecision({
        state: waitingRoomState(),
        workspacePath: pendingWorkspaceTarget(),
        worktreePath: pendingWorktreeTarget(),
        remote,
      })
      if (controlDecision.action !== "none") {
        if (controlDecision.action === "cloud") {
          await handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] })
          return
        }
        if (controlDecision.action === "stage-command") {
          setPromptText(controlDecision.command)
          promptInput?.focus()
          syncCommandCenter(controlDecision.command)
          flashFooter(controlDecision.message, "info")
          return
        }
        if (controlDecision.action === "open-terminal-pairing") {
          await openTerminalPairingDialog()
          return
        }
        if (controlDecision.action === "open-session-browser") {
          openSessionBrowserDialog()
          return
        }
        flashFooter(controlDecision.message, controlDecision.action === "error" ? "error" : "info")
        return
      }
      const decision = deriveWaitingRoomActivationDecision({
        state: waitingRoomState(),
        sessions: availableSessions(),
        catalog: providerCatalogState(),
        currentProvider: (options.provider ?? "opencode") as BackendProviderId,
        currentModel: options.model,
        remote,
      })
      if (decision.action === "create") {
        const session = await createSession(client, pendingWorkspaceTarget(), pendingWorktreeTarget(), undefined, {
          provider: decision.launch.provider,
          model: decision.launch.model,
          effort: decision.launch.effort,
          account_profile: options.accountProfile,
          execution_mode: "build",
          permission_level: "yolo",
        }, decision.launch.sliceRef)
        await attachBinding(session, true, decision.launch)
        flashFooter(`created session ${session.alias ?? session.id}`, "info")
        return
      }
      if (decision.action === "join") {
        await attachBinding(decision.session, false, decision.launch)
        flashFooter(`attached to session ${decision.session.alias ?? decision.session.id}`, "info")
        return
      }
      if (decision.action === "error") {
        flashFooter(decision.message, "error")
      }
    } catch (error) {
      appLogger?.warn("waiting room activation failed", {
        error: formatError(error),
      })
      flashFooter(formatError(error), "error")
    }
  }
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
  const applyWaitingRoomSessionLifecycleAction = async (
    action: WaitingRoomSessionLifecycleAction,
    stateOverride?: WaitingRoomState,
  ) => {
    try {
      if (!kernelConnected()) {
        await connectDetachedKernelFromWaitingRoom()
      }
      const effectiveState = stateOverride ?? waitingRoomState()
      const remote = {
        cloudNotice: waitingRoomCloudNotice(),
        inventoryStatus: waitingRoomInventoryStatus(),
        loadingFrame: waitingRoomState().introStep,
        relay: relayStatusState(),
        machines: remoteMachinesState(),
        kernels: remoteKernelsState(),
        terminals: terminalsState(),
      }
      const decision = action === "delete"
        ? deriveWaitingRoomDeleteDecision({
            state: effectiveState,
            sessions: availableSessions(),
            catalog: providerCatalogState(),
            remote,
          })
        : deriveWaitingRoomSessionLifecycleDecision({
            action,
            state: effectiveState,
            sessions: availableSessions(),
            catalog: providerCatalogState(),
          })
      if (decision.action === "error") {
        waitingRoomLifecycleConfirmationController.clear()
        flashFooter(decision.message, "error")
        return
      }

      const confirmation = waitingRoomLifecycleConfirmationController.confirm(action, decision)
      if (confirmation.action === "await-confirmation") {
        flashFooter(confirmation.message, confirmation.tone)
        return
      }

      if (decision.action === "archive") {
        const updated = await archiveSessionById(client, decision.session.id)
        setAvailableSessions(availableSessions().filter((candidate) => candidate.id !== updated.id))
        waitingRoomInventoryRefreshController.invalidate()
        reconcileWaitingRoom(waitingRoomState())
        await refreshWaitingRoomData()
        flashFooter(`archived session ${formatSessionDisplayLabel(updated)}`, "info")
        return
      }
      if (decision.action === "archive-all") {
        const archived = []
        for (const session of decision.sessions) {
          archived.push(await archiveSessionById(client, session.id))
        }
        const archivedIds = new Set(archived.map((session) => session.id))
        setAvailableSessions(availableSessions().filter((candidate) => !archivedIds.has(candidate.id)))
        waitingRoomInventoryRefreshController.invalidate()
        reconcileWaitingRoom({ ...waitingRoomState(), focus: "new", sessionIndex: 0 })
        await refreshWaitingRoomData()
        if (sessionBrowserOpen()) {
          closeSessionBrowserDialog()
        }
        flashFooter(`archived ${archived.length} session${archived.length === 1 ? "" : "s"}`, "info")
        return
      }
      if (decision.action === "delete-session") {
        const updated = await deleteSessionByRef(client, decision.session.id, pendingWorkspaceTarget())
        setAvailableSessions(availableSessions().filter((candidate) => candidate.id !== updated.id))
        waitingRoomInventoryRefreshController.invalidate()
        reconcileWaitingRoom(waitingRoomState())
        await refreshWaitingRoomData()
        flashFooter(`deleted session ${formatSessionDisplayLabel(updated)}`, "error")
        return
      }
      if (decision.action === "delete-all-sessions") {
        const deleted = []
        for (const session of decision.sessions) {
          deleted.push(await deleteSessionByRef(client, session.id, pendingWorkspaceTarget()))
        }
        const deletedIds = new Set(deleted.map((session) => session.id))
        setAvailableSessions(availableSessions().filter((candidate) => !deletedIds.has(candidate.id)))
        waitingRoomInventoryRefreshController.invalidate()
        reconcileWaitingRoom({ ...waitingRoomState(), focus: "new", sessionIndex: 0 })
        await refreshWaitingRoomData()
        if (sessionBrowserOpen()) {
          closeSessionBrowserDialog()
        }
        flashFooter(`deleted ${deleted.length} session${deleted.length === 1 ? "" : "s"}`, "error")
        return
      }
      if (decision.action === "delete") {
        const updated = await deleteSessionByRef(client, decision.session.id, pendingWorkspaceTarget())
        setAvailableSessions(availableSessions().filter((candidate) => candidate.id !== updated.id))
        waitingRoomInventoryRefreshController.invalidate()
        reconcileWaitingRoom(waitingRoomState())
        await refreshWaitingRoomData()
        flashFooter(`deleted session ${formatSessionDisplayLabel(updated)}`, "error")
        return
      }
      if (decision.action === "delete-machine") {
        const deleted = await forgetRemoteMachine(client, decision.machineId)
        const deletedMachineId = deleted.machine_id || decision.machineId
        setRemoteMachinesState(remoteMachinesState().filter((machine) => machine.machine_id !== deletedMachineId))
        setRemoteKernelsState(remoteKernelsState().filter((kernel) => kernel.machine_id !== deletedMachineId))
        waitingRoomInventoryRefreshController.invalidate()
        reconcileWaitingRoom(waitingRoomState())
        await refreshWaitingRoomData()
        flashFooter(`deleted machine ${decision.label}`, "error")
        return
      }
      if (decision.action === "delete-kernel") {
        hiddenWaitingRoomKernelIds.add(decision.kernelId)
        const hiddenKernelIds = [...hiddenWaitingRoomKernelIds].sort()
        void saveUiPreferences({ hiddenRemoteKernelIds: hiddenKernelIds })
        setPreferencesState((current) => mergeUiPreferences(current, { hiddenRemoteKernelIds: hiddenKernelIds }))
        setRemoteKernelsState(remoteKernelsState().filter((kernel) => kernel.kernel_id !== decision.kernelId))
        reconcileWaitingRoom(waitingRoomState())
        flashFooter(`deleted kernel ${decision.label}`, "error")
      }
    } catch (error) {
      appLogger?.warn("waiting room session lifecycle action failed", {
        action,
        error: formatError(error),
      })
      flashFooter(formatError(error), "error")
    }
  }
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
    if (!isAttached()) {
      return SESSION_NEW_PLACEHOLDER
    }
    return formatWorkflowPromptPlaceholder({
      workflowScreenActive: workflowScreenShowing(),
      state: workflowPromptState(),
      attachedPlaceholder: ATTACHED_PROMPT_PLACEHOLDER,
      detachedPlaceholder: SESSION_NEW_PLACEHOLDER,
    })
  }
  const promptAreaBackground = () => (
    themeRevision(),
    isAttached()
      ? (workflowScreenShowing() ? theme.backgroundElement : theme.backgroundPanel)
      : theme.backgroundElement
  )
  const restorePromptHistory = (sessionId: string | null) => {
    const preferences = untrack(preferencesState)
    const nextEntries = sessionId
      ? sessionPromptHistoryEntries(preferences, sessionId)
      : []
    const nextDraft = sessionId
      ? sessionPromptDraftEntry(preferences, sessionId)
      : ""
    setPromptHistoryEntries(nextEntries)
    setPromptHistoryIndex(null)
    setPromptHistoryDraft(null)
    setPromptText(nextDraft)
  }
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
  const promptInputMaxHeight = () => (
    isAttached()
      ? Math.max(6, dimensions().height - 11)
      : 6
  )
  const retainPromptFocus = () => {
    if (!isAttached()) {
      return
    }
    startTimeout(() => {
      promptInput?.focus()
    }, 0)
  }
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
  const syncPromptPlaceholder = () => {
    if (!promptInput) {
      return
    }
    promptInput.placeholder = promptPlaceholder()
  }
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
  createEffect(() => {
    const attachedSessionId = attachmentState()?.session_id ?? null
    if (attachedSessionId === hydratedPromptHistorySessionId) {
      return
    }
    hydratedPromptHistorySessionId = attachedSessionId
    restorePromptHistory(attachedSessionId)
    if (!attachedSessionId) {
      promptHistoryHydrationController.invalidate()
      return
    }
    void hydratePromptHistoryFromSession(attachedSessionId).catch((error) => {
      if (attachmentState()?.session_id !== attachedSessionId) {
        return
      }
      appLogger?.warn("failed to hydrate prompt history from session history", {
        session_id: attachedSessionId,
        error: formatError(error),
      })
    })
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

  const applyVisibleTranscriptState = (
    nextEntries: TranscriptEntry[],
    agentId: string | null | undefined = visibleTranscriptAgentId(),
    turnIds = expandedTurnIdsForAgent(agentId),
  ) => {
    const preparedEntries = applyTranscriptDisplayState(nextEntries, turnIds)
    setEntries(reconcile(preparedEntries))
    setEntryCounter(preparedEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    return preparedEntries
  }

  const findTurnAnchorEntryId = (turnId: number) => {
    return entries.find((entry) => entry.turnId === turnId && entry.role === "user")?.id
      ?? entries.find((entry) => entry.turnId === turnId && entry.role !== "turn_toggle")?.id
      ?? null
  }

  const toggleTurn = (turnId: number | null | undefined, toggleEntryId?: number) => {
    if (!turnId) {
      return
    }
    const currentEntries = entries.filter(Boolean)
    const toggleEntry = resolveVisibleTurnToggle(currentEntries, turnId, toggleEntryId)
    if (!toggleEntry) {
      return
    }
    const agentId = visibleTranscriptAgentId()
    const expanding = toggleEntry?.toggleMode === "expand"
    setExpandedTurnState(agentId, turnId, expanding)
    const nextEntries = applyTranscriptDisplayState(currentEntries, expanding
      ? expandedTurnIdsForAgent(agentId).filter((value) => value !== turnId)
      : [...expandedTurnIdsForAgent(agentId), turnId])
    setEntries(reconcile(nextEntries))
    setEntryCounter(nextEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    persistVisibleTranscriptEntries(nextEntries)
    reconcileMountedTranscript(currentEntries, nextEntries)
    retainPromptFocus()
  }

  const toggleBlob = (entryId: number, collapsed: boolean) => {
    const currentEntries = entries.filter(Boolean)
    const agentId = visibleTranscriptAgentId()
    const nextEntries = setTranscriptBlobCollapsed(currentEntries, entryId, expandedTurnIdsForAgent(agentId), collapsed)
    setEntries(reconcile(nextEntries))
    setEntryCounter(nextEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    persistVisibleTranscriptEntries(nextEntries)
    reconcileMountedTranscript(currentEntries, nextEntries)
    retainPromptFocus()
  }

  const appendEntry = (entry: Omit<TranscriptEntry, "id">, turnIds = expandedTurnIdsForAgent(visibleTranscriptAgentId())) => {
    const previousEntry = entries.at(-1)
    if (shouldSkipConsecutiveTranscriptEntry(previousEntry, entry)) {
      return
    }
    const currentEntries = entries.filter(Boolean)
    const nextId = entryCounter() + 1
    const nextEntry: TranscriptEntry = { id: nextId, ...entry }
    if (nextEntry.turnId === undefined && currentTurnId !== null) {
      nextEntry.turnId = currentTurnId
    }
    const nextEntries = applyVisibleTranscriptState([...currentEntries, nextEntry], visibleTranscriptAgentId(), turnIds)
    persistVisibleTranscriptEntries(nextEntries)
    reconcileMountedTranscript(currentEntries, nextEntries)
    enforceTranscriptRetention()
  }

  const scrollTranscriptToBottom = () => {
    if (!transcriptScrollbox) {
      return
    }
    historyScrollRestoreController.cancel()
    const maxScrollTop = Math.max(0, transcriptScrollbox.scrollHeight - transcriptScrollbox.height)
    transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: maxScrollTop })
    transcriptScrollbox.requestRender()
    lastTranscriptScrollTop = transcriptScrollbox.scrollTop
  }

  const trackAgentFocusTransition = <T,>(operation: () => Promise<T>): Promise<T> =>
    agentFocusTransitionController.track(operation)

  const waitForPendingAgentFocusTransition = (): Promise<void> =>
    agentFocusTransitionController.wait()

  const appendUserPrompt = (text: string, agentId?: string | null) => {
    recordTurnActivity("prompt_submit")
    turnCompletionController.reset()
    const targetAgentId = agentId ?? focusedAgentId()
    submittingAgentId = targetAgentId
    setStreamingAgentId(targetAgentId)
    markAgentBusy(targetAgentId)
    if (splitAgentResponseMode() && targetAgentId && targetAgentId !== responsePrimaryAgent()?.id) {
      const paneEntries = currentAgentPaneEntries(targetAgentId)
      const nextTurnIds = collapseLatestTurnForAgent(targetAgentId, paneEntries)
      appendTranscriptEntryToAgentPane(targetAgentId, {
        role: "user",
        text: trimSingleTrailingNewline(text),
        turnId: computeNextTurnId(paneEntries),
      }, nextTurnIds)
      setSubmitting(true)
      setWorking(true)
      renderSessionChromeBoundary()
      return
    }
    const turnId = nextTurnId
    nextTurnId += 1
    currentTurnId = turnId
    const nextTurnIds = collapseLatestTurnForAgent(targetAgentId, entries.filter(Boolean))
    appendEntry({ role: "user", text: trimSingleTrailingNewline(text), turnId }, nextTurnIds)
    syncVisibleTranscriptPreview()
    setSubmitting(true)
    setWorking(true)
    renderSessionChromeBoundary()
    scrollTranscriptToBottom()
  }

  const appendNotice = (text: string, emphasis: TranscriptEntry["emphasis"] = "muted") => {
    appendEntry({ role: "notice", text, emphasis })
    syncVisibleTranscriptPreview()
    updateSessionChrome()
  }

  const appendCloudNotice = (text: string) => {
    if (isAttached()) {
      appendNotice(text)
      return
    }
    setWaitingRoomCloudNotice(text)
    rebuildTranscript()
    updateSessionChrome()
  }

  const appendProviderError = (text: string) => {
    const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
    if (!normalized) {
      return
    }
    cancelPendingTurnCompletion()
    setWorking(false)
    setSubmitting(false)
    clearAgentBusy(visibleTranscriptAgentId())
    submittingAgentId = null
    appendEntry({ role: "error", text: normalized, emphasis: "error" })
    syncVisibleTranscriptPreview()
    renderSessionChromeBoundary()
    scrollTranscriptToBottom()
  }

  const removeTranscriptRenderable = (entryId: number) => {
    const renderable = transcriptRenderables.get(entryId)
    if (!renderable || !transcriptScrollbox) {
      return
    }
    transcriptScrollbox.remove(renderable.wrapper.id)
    renderable.wrapper.destroyRecursively()
    transcriptRenderables.delete(entryId)
  }

  const enforceTranscriptRetention = () => {
    const currentEntries = entries.slice()
    let totalChars = currentEntries.reduce((sum, entry) => sum + entry.text.length, 0)
    let removeCount = 0

    while (
      currentEntries.length - removeCount > LIVE_TRANSCRIPT_LIMIT
      || (totalChars > LIVE_TRANSCRIPT_MAX_CHARS && removeCount < currentEntries.length - 1)
    ) {
      totalChars -= currentEntries[removeCount]?.text.length ?? 0
      removeCount += 1
    }

    if (removeCount === 0) {
      return
    }

    const removed = currentEntries.slice(0, removeCount)
    const kept = currentEntries.slice(removeCount)
    for (const entry of removed) {
      removeTranscriptRenderable(entry.id)
      if (entry.mergeKey) {
        tools.delete(entry.mergeKey)
      }
    }
    setEntries(reconcile(kept))
    transcriptScrollbox?.requestRender()
  }

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

  const clipboardController = createClipboardController({
    renderer,
    promptInput: () => promptInput ?? null,
    flashFooter,
    logWarning: (message, fields) => appLogger?.warn(message, fields),
    formatError,
  })
  const copyPromptSelection = clipboardController.copyPromptSelection
  const copySelection = clipboardController.copySelection

  const applySessionState = (nextSession: RuntimeSession) => {
    nextSession = normalizeRuntimeSession(nextSession)
    const previousFocusedAgentId = focusedAgentId()
    const previousLayout = multiAgentResponseLayout()
    const promptLifecycle = derivePromptLifecycleTransition(sessionState(), nextSession)
    const transition = deriveSessionTransitionState({
      currentSession: sessionState(),
      nextSession,
      currentWorking: working(),
      currentStreamingAgentId: streamingAgentId(),
      currentAgentActivityLabels: agentActivityLabels(),
      layoutPreference: preferencesState().ui?.multiAgentResponseLayout,
    })
    const shouldConfirmIdleCompletion = shouldConfirmIdleTurnCompletion({
      nextSession,
      currentWorking: working(),
      currentSubmitting: submitting(),
      currentBusyLatches: agentBusyLatches(),
      currentStreamingAgentId: streamingAgentId(),
      currentProviderActivityLabel: providerActivityLabel(),
      currentActiveStatusLabel: activeStatusLabel(),
    })
    setSessionState(nextSession)
    setAgentActivityLabels(transition.nextAgentActivityLabels)
    setStreamingAgentId(transition.nextStreamingAgentId)
    setMultiAgentResponseLayout(transition.nextLayout)
    setWorking(transition.nextWorking)
    if (transition.nextHasPromptWork) {
      turnCompletionController.reset()
    } else if (turnCompletionController.isConfirmed() || shouldConfirmIdleCompletion) {
      turnCompletionController.confirmAndSchedule()
    } else {
      cancelPendingTurnCompletion()
    }
    setProviderActivityLabel(transition.nextFocusedActivityLabel)
    setActiveStatusLabel(transition.nextFocusedActivityLabel)
    if (promptLifecycle.activePromptChanged) {
      setSubmitting(false)
      submittingAgentId = null
      promptStopController.reset()
    }
    for (const settledAgentId of promptLifecycle.settledAgentIds) {
      clearAgentBusy(settledAgentId)
    }
    if (promptLifecycle.cancelledPromptSettled) {
      activeToolLabels.clear()
      setAgentActivityLabels({})
      setStreamingAgentId(nextSession.active_prompt?.target_agent_id ?? null)
      setProviderActivityLabel(null)
      setActiveStatusLabel(null)
      if (statusLine() === "Cancellation requested.") {
        setStatusLine(DEFAULT_CONNECTED_STATUS)
      }
      if (!transition.nextHasPromptWork) {
        turnCompletionController.confirm()
        cancelPendingTurnCompletion()
        setWorking(false)
      }
    }
    if (!transition.nextHasPromptWork) {
      setSubmitting(false)
      promptStopController.reset()
    }
    syncVisibleActivityLabel()
    updateSessionChrome()
    if (
      transition.nextLayout === "split"
      && (previousLayout !== transition.nextLayout
        || previousFocusedAgentId !== transition.nextFocusedAgentId
        || transition.previousAgentSignature !== transition.nextAgentSignature)
    ) {
      refreshSplitPaneFocusRepaint()
    }
  }

  const clearLocalBusyStateForAuthoritativeIdle = (nextSession: RuntimeSession) => {
    if (sessionHasPromptWork(nextSession) || sessionHasProcessingAgent(nextSession)) {
      return
    }
    batch(() => {
      turnCompletionController.reset()
      activeToolLabels.clear()
      setAgentActivityLabels({})
      setStreamingAgentId(null)
      setSubmitting(false)
      submittingAgentId = null
      promptStopController.reset()
      setAgentBusyLatches({})
      setProviderActivityLabel(null)
      setActiveStatusLabel(null)
      setWorking(false)
      if (statusLine() === "Cancellation requested.") {
        setStatusLine(DEFAULT_CONNECTED_STATUS)
      }
    })
    renderSessionChromeBoundary()
  }

  const applyProviderActivity = (active: boolean) => {
    if (active) {
      setWorking(true)
    }
    turnCompletionController.handleProviderActivity(active)
    updateSessionChrome()
  }

  const markAssistantMessageCompleted = (_agentId: string | null | undefined) => {
    const completionAgentId = _agentId ?? visibleTranscriptAgentId()
    const turnId = completionAgentId && splitAgentResponseMode() && completionAgentId !== visibleTranscriptAgentId()
      ? computeCurrentTurnId(currentAgentPaneEntries(completionAgentId))
      : computeCurrentTurnId(entries.filter(Boolean))
    if (completionAgentId && turnId !== null) {
      const nextExpandedTurnIds = [...new Set([...expandedTurnIdsForAgent(completionAgentId), turnId])]
        .filter((value) => value !== turnId)
        .sort((left, right) => left - right)
      setExpandedTurnIdsByAgent((current) => ({
        ...current,
        [completionAgentId]: nextExpandedTurnIds,
      }))
      if (completionAgentId === visibleTranscriptAgentId()) {
        const currentEntries = entries.filter(Boolean)
        const nextEntries = applyTranscriptDisplayState(currentEntries, nextExpandedTurnIds)
        setEntries(reconcile(nextEntries))
        setEntryCounter(nextEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
        persistVisibleTranscriptEntries(nextEntries)
        reconcileMountedTranscript(currentEntries, nextEntries)
      } else {
        setAgentTranscriptEntries(completionAgentId, currentAgentPaneEntries(completionAgentId))
      }
    }
    clearAgentBusy(completionAgentId)
    turnCompletionController.confirm()
    maybeScheduleConfirmedTurnCompletion()
  }

  const syncVisibleActivityLabel = () => {
    setActiveStatusLabel(focusedActivityLabel())
  }

  const syncActiveToolLabel = (update: ToolTranscriptUpdate) => {
    const label = getToolActivityLabel(update.tool)
    const terminal = update.status === "completed" || update.status === "error" || update.status === "cancelled"

    activeToolLabels.delete(update.id)
    if (label && !terminal) {
      activeToolLabels.set(update.id, label)
    }

    syncVisibleActivityLabel()
  }

  const appendProviderChunk = (
    role: TranscriptEntry["role"],
    chunk: string,
    mergeKey?: string,
    sourceText?: string,
  ) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    const normalizedSource = sourceText?.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }
    cancelPendingTurnCompletion()
    setWorking(true)
    setSubmitting(false)
    const currentEntries = entries.filter(Boolean).map((entry) => ({ ...entry }))
    const nextEntries = currentEntries.map((entry) => ({ ...entry }))
    let merged = false
    let mergedEntryId: number | null = null
    let mergedEntryText: string | null = null
    let mergedEntrySourceText: string | undefined

    if (mergeKey) {
      for (let index = nextEntries.length - 1; index >= 0; index -= 1) {
        const candidate = nextEntries[index]
        if (candidate?.role !== role || candidate.mergeKey !== mergeKey) {
          continue
        }
        if (role === "assistant" || role === "reasoning") {
          candidate.text += normalized
          if (normalizedSource !== undefined) {
            candidate.sourceText = `${candidate.sourceText ?? ""}${normalizedSource}`
          }
        } else {
          candidate.text = normalized
          if (normalizedSource !== undefined) {
            candidate.sourceText = normalizedSource
          }
        }
        merged = true
        mergedEntryId = candidate.id
        mergedEntryText = candidate.text
        mergedEntrySourceText = candidate.sourceText
        break
      }
    }

    if (!merged) {
      const last = [...nextEntries].reverse().find((entry) => entry.role !== "turn_toggle")
      if (!mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
        last.text += normalized
        merged = true
        mergedEntryId = last.id
        mergedEntryText = last.text
        mergedEntrySourceText = last.sourceText
      }
    }

    if (merged && mergedEntryId !== null && mergedEntryText !== null) {
      setEntries(reconcile(nextEntries))
      persistVisibleTranscriptEntries(nextEntries)
      updateTranscriptEntry(mergedEntryId, mergedEntryText, mergedEntrySourceText)
      logVisibleTranscriptOutput(role, mergedEntryText, true, mergeKey)
      enforceTranscriptRetention()
      maybeScheduleConfirmedTurnCompletion()
      return
    }

    if (!merged) {
      const nextEntry: TranscriptEntry = {
        id: entryCounter() + 1,
        role,
        text: normalized,
      }
      if (currentTurnId !== null) {
        nextEntry.turnId = currentTurnId
      }
      if (mergeKey) {
        nextEntry.mergeKey = mergeKey
      }
      if (normalizedSource !== undefined) {
        nextEntry.sourceText = normalizedSource
      }
      nextEntries.push(nextEntry)
    }

    const preparedEntries = applyVisibleTranscriptState(nextEntries)
    persistVisibleTranscriptEntries(preparedEntries)
    reconcileMountedTranscript(currentEntries, preparedEntries)
    const loggedEntry = [...preparedEntries].reverse().find((entry) => entry.role === role && (mergeKey ? entry.mergeKey === mergeKey : true))
    logVisibleTranscriptOutput(role, loggedEntry?.text ?? normalized, merged, mergeKey)
    enforceTranscriptRetention()
    maybeScheduleConfirmedTurnCompletion()
  }

  const appendToolUpdate = (chunk: string) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }
    cancelPendingTurnCompletion()
    setWorking(true)
    updateSessionChrome()
    const parsed = parseToolTranscriptUpdate(normalized)
    if (parsed) {
      const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
      tools.set(parsed.id, merged)
      syncActiveToolLabel(merged)
      appendProviderChunk("tool", formatToolTranscriptUpdate(merged), parsed.id, JSON.stringify(merged))
      return
    }
    appendProviderChunk("tool", normalized, undefined, normalized)
  }

  const processTerminalOutputRecord = (record: TerminalOutputRecord) => {
    if (record.kind === "prompt_echo") {
      appendPromptEchoToSharedHistory(Buffer.from(record.bytes).toString("utf8"))
    }
    kernelEventController.processTerminalOutputRecord(record)
  }

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

  const setTextRenderable = (
    text: TextRenderable | undefined,
    content: string,
    fg: (typeof theme)[keyof typeof theme],
    attributes = TextAttributes.NONE,
  ) => {
    if (!text) {
      return
    }
    text.content = content
    text.fg = fg
    text.attributes = attributes
  }

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
    if (!responseLayoutBox || !primaryPane) {
      logViewDebug("apply response layout:missing refs", {
        has_layout_box: Boolean(responseLayoutBox),
        has_primary_pane: Boolean(primaryPane),
        auxiliary_pane_count: responseAuxiliaryPanes.filter(Boolean).length,
      })
      return
    }

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

    responseLayoutBox.flexDirection = "column"
    responseLayoutBox.gap = 0

    const borderColor = (tone: PaneGridTone) => tone === "focused" ? theme.primary : theme.borderSubtle

    const layoutPane = (
      pane: BoxRenderable | undefined,
      interactionBox: BoxRenderable | undefined,
      footerBox: BoxRenderable | undefined,
      scrollbox: ScrollBoxRenderable | undefined,
      focused: boolean,
      visible: boolean,
      showFooter: boolean,
      defaultBackground: RGBA,
    ) => {
      if (!pane) {
        return
      }
      pane.visible = visible
      pane.flexDirection = "column"
      pane.flexGrow = visible ? 1 : 0
      pane.flexBasis = visible ? 0 : 0
      pane.width = visible ? "auto" : 0
      pane.minWidth = visible && split ? 0 : null
      pane.maxWidth = null
      pane.paddingLeft = 0
      pane.paddingRight = 0
      pane.paddingTop = 0
      pane.paddingBottom = 0
      pane.border = false
      pane.borderColor = theme.borderSubtle
      pane.backgroundColor = visible && split
        ? transcriptSurfacePalette(resolveTranscriptSurfaceTone(true, focused)).panel
        : defaultBackground
      interactionBox && (interactionBox.visible = visible && Boolean(interactionBox.getChildren().length))
      footerBox && (footerBox.visible = visible && showFooter)
      if (scrollbox) {
        scrollbox.backgroundColor = pane.backgroundColor
        scrollbox.requestRender?.()
      }
      interactionBox?.requestRender?.()
      pane.requestRender?.()
      footerBox?.requestRender?.()
    }

    const applyBorderRowBox = (box: BoxRenderable | undefined, visible: boolean) => {
      if (!box) {
        return
      }
      box.visible = visible
      box.height = visible ? 1 : 0
      box.minHeight = visible ? 1 : 0
      box.flexGrow = 0
      box.flexShrink = 0
      box.flexDirection = "row"
      box.gap = 0
      box.requestRender?.()
    }

    const applyHorizontalSegment = (
      segmentBox: BoxRenderable | undefined,
      visible: boolean,
      tone: PaneGridTone,
    ) => {
      if (!segmentBox) {
        return
      }
      segmentBox.visible = visible
      segmentBox.height = 1
      segmentBox.minHeight = 1
      segmentBox.flexGrow = visible ? 1 : 0
      segmentBox.flexBasis = 0
      segmentBox.border = visible ? ["top"] : false
      segmentBox.borderColor = borderColor(tone)
      segmentBox.requestRender?.()
    }

    const applyVerticalSegment = (
      segmentBox: BoxRenderable | undefined,
      visible: boolean,
      tone: PaneGridTone,
    ) => {
      if (!segmentBox) {
        return
      }
      segmentBox.visible = visible
      segmentBox.width = visible ? 1 : 0
      segmentBox.minWidth = visible ? 1 : 0
      segmentBox.flexGrow = 0
      segmentBox.flexShrink = 0
      segmentBox.border = visible ? ["left"] : false
      segmentBox.borderColor = borderColor(tone)
      segmentBox.requestRender?.()
    }

    const applyJunctionText = (
      text: TextRenderable | undefined,
      visible: boolean,
      char: string,
      tone: PaneGridTone,
    ) => {
      setTextRenderable(text, visible ? char : "", borderColor(tone))
    }

    paneGrid.rows.forEach((gridRow, rowIndex) => {
      const rowBox = responseRowBoxes[rowIndex]
      if (!rowBox) {
        return
      }
      const borderRow = paneGrid.borderRows[rowIndex]
      if (borderRow) {
        applyBorderRowBox(paneGridBorderRows[rowIndex], borderRow.visible)
        borderRow.horizontals.forEach((segment, segmentIndex) => {
          applyHorizontalSegment(
            paneGridHorizontalSegments[rowIndex]?.[segmentIndex],
            segment.visible,
            segment.tone,
          )
        })
        borderRow.junctions.forEach((junction, junctionIndex) => {
          applyJunctionText(
            paneGridJunctionTexts[rowIndex]?.[junctionIndex],
            junction.visible,
            junction.char,
            junction.tone,
          )
        })
      }

      rowBox.visible = rowIndex === 0 || gridRow.visible
      rowBox.flexDirection = "row"
      rowBox.gap = 0
      rowBox.flexGrow = rowBox.visible ? 1 : 0
      rowBox.flexBasis = 0
      rowBox.border = false
      rowBox.requestRender?.()

      gridRow.verticals.forEach((segment, segmentIndex) => {
        applyVerticalSegment(
          paneGridVerticalSegments[rowIndex]?.[segmentIndex],
          segment.visible,
          segment.tone,
        )
      })

      for (const slot of gridRow.slots) {
        if (slot.paneIndex === 0) {
          layoutPane(
            primaryPane,
            responsePrimaryInteractionBox,
            responsePrimaryFooterBox,
            transcriptScrollbox,
            slot.focused,
            true,
            !showWorkflowScreen,
            theme.backgroundPanel,
          )
          if (historyLoadingBox) {
            historyLoadingBox.backgroundColor = primaryPane.backgroundColor
            historyLoadingBox.borderColor = split && slot.focused ? theme.primary : theme.borderSubtle
            historyLoadingBox.requestRender?.()
          }
          continue
        }
        const auxiliaryIndex = slot.paneIndex - 1
        layoutPane(
          responseAuxiliaryPanes[auxiliaryIndex],
          responseAuxiliaryInteractionBoxes[auxiliaryIndex],
          responseAuxiliaryFooterBoxes[auxiliaryIndex],
          responseAuxiliaryScrollboxes[auxiliaryIndex],
          slot.focused,
          true,
          Boolean(slot.agentId),
          theme.backgroundElement,
        )
      }

      for (const paneIndex of paneRows[rowIndex] ?? []) {
        if (gridRow.slots.some((slot) => slot.paneIndex === paneIndex)) {
          continue
        }
        if (paneIndex === 0) {
          layoutPane(
            primaryPane,
            responsePrimaryInteractionBox,
            responsePrimaryFooterBox,
            transcriptScrollbox,
            false,
            false,
            false,
            theme.backgroundPanel,
          )
          continue
        }
        const auxiliaryIndex = paneIndex - 1
        layoutPane(
          responseAuxiliaryPanes[auxiliaryIndex],
          responseAuxiliaryInteractionBoxes[auxiliaryIndex],
          responseAuxiliaryFooterBoxes[auxiliaryIndex],
          responseAuxiliaryScrollboxes[auxiliaryIndex],
          false,
          false,
          false,
          theme.backgroundElement,
        )
      }
    })

    const bottomBorderRow = paneGrid.borderRows[paneGrid.rows.length]
    if (bottomBorderRow) {
      applyBorderRowBox(paneGridBottomBorderRow, bottomBorderRow.visible)
      bottomBorderRow.horizontals.forEach((segment, segmentIndex) => {
        applyHorizontalSegment(
          paneGridBottomHorizontalSegments[segmentIndex],
          segment.visible,
          segment.tone,
        )
      })
      bottomBorderRow.junctions.forEach((junction, junctionIndex) => {
        applyJunctionText(
          paneGridBottomJunctionTexts[junctionIndex],
          junction.visible,
          junction.char,
          junction.tone,
        )
      })
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

  const setExpandedTurnState = (agentId: string | null | undefined, turnId: number | null | undefined, expanded: boolean) => {
    if (!agentId || !turnId) {
      return
    }
    setExpandedTurnIdsByAgent((current) => {
      const previous = new Set(current[agentId] ?? [])
      if (expanded) {
        previous.delete(turnId)
      } else {
        previous.add(turnId)
      }

      if (previous.size === 0) {
        if (!(agentId in current)) {
          return current
        }
        const next = { ...current }
        delete next[agentId]
        return next
      }

      const nextTurnIds = [...previous].sort((left, right) => left - right)
      const currentTurnIds = current[agentId] ?? []
      if (currentTurnIds.length === nextTurnIds.length && currentTurnIds.every((value, index) => value === nextTurnIds[index])) {
        return current
      }
      return {
        ...current,
        [agentId]: nextTurnIds,
      }
    })
  }

  const replaceExpandedTurnsForAgent = (agentId: string | null | undefined, turnIds: readonly number[]) => {
    if (!agentId) {
      return
    }
    setExpandedTurnIdsByAgent((current) => {
      const nextTurnIds = [...new Set(turnIds)].sort((left, right) => left - right)
      if (nextTurnIds.length === 0) {
        if (!(agentId in current)) {
          return current
        }
        const next = { ...current }
        delete next[agentId]
        return next
      }

      const currentTurnIds = current[agentId] ?? []
      if (currentTurnIds.length === nextTurnIds.length && currentTurnIds.every((value, index) => value === nextTurnIds[index])) {
        return current
      }
      return {
        ...current,
        [agentId]: nextTurnIds,
      }
    })
  }

  const collapseLatestTurnForAgent = (agentId: string | null | undefined, paneEntries: TranscriptEntry[]) => {
    const nextTurnIds = collapseLatestTranscriptTurn(paneEntries, expandedTurnIdsForAgent(agentId))
    replaceExpandedTurnsForAgent(agentId, nextTurnIds)
    return nextTurnIds
  }

  const applyExpandedTurns = (entries: TranscriptEntry[], expandedTurnIds: readonly number[]) => {
    return applyTranscriptDisplayState(entries, expandedTurnIds)
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

  const auxiliaryAgentPaneRenderables = (agentId: string) => {
    let renderables = agentTranscriptRenderables.get(agentId)
    if (!renderables) {
      renderables = new Map<number, TranscriptEntryRenderable>()
      agentTranscriptRenderables.set(agentId, renderables)
    }
    return renderables
  }

  const auxiliaryAgentPaneTools = (agentId: string) => {
    let toolState = agentPaneTools.get(agentId)
    if (!toolState) {
      toolState = new Map<string, ToolTranscriptUpdate>()
      agentPaneTools.set(agentId, toolState)
    }
    return toolState
  }

  const currentAgentPaneEntries = (agentId: string) => {
    return selectCurrentAgentPaneEntries({
      agentId,
      visibleAgentId: visibleTranscriptAgentId(),
      visibleEntries: entries.filter(Boolean),
      paneEntriesByAgent: agentPaneEntries(),
    })
  }

  const hasTrailingUserPrompt = (agentId: string, text: string) => {
    const lastEntry = currentAgentPaneEntries(agentId).at(-1)
    return lastEntry?.role === "user" && trimSingleTrailingNewline(lastEntry.text) === trimSingleTrailingNewline(text)
  }

  const toggleAuxiliaryPaneTurn = (agentId: string, turnId: number | null | undefined, toggleEntryId?: number) => {
    if (!turnId) {
      return
    }
    const currentEntries = currentAgentPaneEntries(agentId)
    const toggleEntry = resolveVisibleTurnToggle(currentEntries, turnId, toggleEntryId)
    if (!toggleEntry) {
      return
    }
    const expanding = toggleEntry?.toggleMode === "expand"
    setExpandedTurnState(agentId, turnId, expanding)
    const nextEntries = applyTranscriptDisplayState(currentEntries, expanding
      ? expandedTurnIdsForAgent(agentId).filter((value) => value !== turnId)
      : [...expandedTurnIdsForAgent(agentId), turnId])
    commitAgentPaneEntries(agentId, nextEntries)
    reconcileMountedAuxiliaryTranscript(agentId, currentEntries, nextEntries)
    retainPromptFocus()
  }

  const toggleAuxiliaryPaneBlob = (agentId: string, entryId: number, collapsed: boolean) => {
    const currentEntries = currentAgentPaneEntries(agentId)
    const nextEntries = setTranscriptBlobCollapsed(currentEntries, entryId, expandedTurnIdsForAgent(agentId), collapsed)
    commitAgentPaneEntries(agentId, nextEntries)
    reconcileMountedAuxiliaryTranscript(agentId, currentEntries, nextEntries)
    retainPromptFocus()
  }

  const clearAuxiliaryAgentPane = (agentId: string) => {
    const scrollbox = agentTranscriptScrollboxes.get(agentId)
    if (scrollbox) {
      for (const child of [...scrollbox.getChildren()]) {
        scrollbox.remove(child.id)
        child.destroyRecursively()
      }
      scrollbox.requestRender()
    }
    agentTranscriptRenderables.delete(agentId)
    agentEmptyTranscriptRenderables.delete(agentId)
  }

  const rebuildAuxiliaryAgentPane = (agentId: string) => {
    const scrollbox = agentTranscriptScrollboxes.get(agentId)
    if (!scrollbox) {
      return
    }

    clearAuxiliaryAgentPane(agentId)

    const paneEntries = agentPaneEntries()[agentId] ?? []
    if (paneEntries.length === 0) {
      const empty = buildEmptyTranscriptRenderable(renderer)
      agentEmptyTranscriptRenderables.set(agentId, empty)
      scrollbox.add(empty)
      scrollbox.requestRender()
      return
    }

    const renderables = auxiliaryAgentPaneRenderables(agentId)
    const surfaceTone = auxiliaryTranscriptSurfaceTone(agentId)
    for (const entry of paneEntries.filter((candidate) => !candidate.historyDeferred)) {
      const renderable = buildTranscriptEntryRenderable(
        renderer,
        entry,
        transcriptSyntax,
        (turnId, nextToggleEntryId) => toggleAuxiliaryPaneTurn(agentId, turnId, nextToggleEntryId),
        (entryId, collapsed) => toggleAuxiliaryPaneBlob(agentId, entryId, collapsed),
        surfaceTone,
      )
      renderables.set(entry.id, renderable)
      scrollbox.add(renderable.wrapper)
    }
    scrollbox.requestRender()
  }

  const mountAuxiliaryTranscriptEntry = (agentId: string, entry: TranscriptEntry, requestRender = true) => {
    const scrollbox = agentTranscriptScrollboxes.get(agentId)
    if (!scrollbox) {
      return
    }

    const empty = agentEmptyTranscriptRenderables.get(agentId)
    if (empty) {
      scrollbox.remove(empty.id)
      empty.destroyRecursively()
      agentEmptyTranscriptRenderables.delete(agentId)
    }

    const renderable = buildTranscriptEntryRenderable(
      renderer,
      entry,
      transcriptSyntax,
      (turnId, nextToggleEntryId) => toggleAuxiliaryPaneTurn(agentId, turnId, nextToggleEntryId),
      (entryId, collapsed) => toggleAuxiliaryPaneBlob(agentId, entryId, collapsed),
      auxiliaryTranscriptSurfaceTone(agentId),
    )
    auxiliaryAgentPaneRenderables(agentId).set(entry.id, renderable)
    scrollbox.add(renderable.wrapper)
    if (requestRender) {
      scrollbox.requestRender()
    }
  }

  const updateAuxiliaryTranscriptEntry = (agentId: string, nextEntry: TranscriptEntry) => {
    const renderable = auxiliaryAgentPaneRenderables(agentId).get(nextEntry.id)
    if (!renderable) {
      rebuildAuxiliaryAgentPane(agentId)
      return
    }
    const previousMode = transcriptRenderMode(renderable.entry)
    if (transcriptRenderMode(nextEntry) !== previousMode) {
      rebuildAuxiliaryAgentPane(agentId)
      return
    }
    renderable.entry = nextEntry
    renderable.update(nextEntry)
    renderScheduler.requestRenderable(agentTranscriptScrollboxes.get(agentId))
  }

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

  const reconcileMountedAuxiliaryTranscript = (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
  ) => {
    reconcileMountedTranscriptPane({
      scrollbox: agentTranscriptScrollboxes.get(agentId),
      currentEntries,
      nextEntries,
      renderables: auxiliaryAgentPaneRenderables(agentId),
      clampScrollTop,
      rebuild: () => rebuildAuxiliaryAgentPane(agentId),
      removeEmptyRenderable: () => {
        const empty = agentEmptyTranscriptRenderables.get(agentId)
        if (!empty) {
          return
        }
        agentTranscriptScrollboxes.get(agentId)?.remove(empty.id)
        empty.destroyRecursively()
        agentEmptyTranscriptRenderables.delete(agentId)
      },
      mountEntry: (entry, requestRender) => mountAuxiliaryTranscriptEntry(agentId, entry, requestRender),
    })
  }

  const pruneAuxiliaryAgentPanes = (session: RuntimeSession) => {
    const activeAgentIds = new Set(
      splitPaneAuxiliaryAgentIds(
        session.agents,
        session.focused_agent_id,
        true,
        maxAgentsPerScreen(),
      ),
    )
    for (const agentId of agentTranscriptScrollboxes.keys()) {
      if (!activeAgentIds.has(agentId)) {
        agentTranscriptScrollboxes.delete(agentId)
      }
    }
    for (const agentId of agentTranscriptRenderables.keys()) {
      if (!activeAgentIds.has(agentId)) {
        agentTranscriptRenderables.delete(agentId)
      }
    }
    for (const agentId of agentEmptyTranscriptRenderables.keys()) {
      if (!activeAgentIds.has(agentId)) {
        agentEmptyTranscriptRenderables.delete(agentId)
      }
    }
    for (const agentId of agentPaneTools.keys()) {
      if (!activeAgentIds.has(agentId)) {
        agentPaneTools.delete(agentId)
      }
    }
  }

  const syncVisibleTranscriptPreview = (
    agentId: string | null = visibleTranscriptAgentId(),
    previewEntries: readonly TranscriptEntry[] = entries.filter(Boolean),
  ) => {
    if (!agentId) {
      return
    }
    setAgentPanePreview(agentId, formatTranscriptPreview([...previewEntries]))
  }

  const appendAgentPanePreview = (agentId: string | null | undefined, line: string) => {
    if (!agentId) {
      return
    }
    const normalized = line.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
    if (!normalized) {
      return
    }
    setAgentPanePreviews((current) => ({
      ...current,
      [agentId]: appendPreviewLine(current[agentId] ?? "", normalized),
    }))
  }

  const appendTranscriptEntryToAgentPane = (
    agentId: string,
    entry: Omit<TranscriptEntry, "id">,
    turnIds = expandedTurnIdsForAgent(agentId),
  ) => {
    const currentEntries = currentAgentPaneEntries(agentId).map((item) => ({ ...item }))
    const previousEntry = currentEntries.at(-1)
    if (shouldSkipConsecutiveTranscriptEntry(previousEntry, entry)) {
      return
    }
    const nextEntry: TranscriptEntry = {
      id: currentEntries.reduce((max, current) => Math.max(max, current.id), 0) + 1,
      ...entry,
    }
    if (nextEntry.turnId === undefined) {
      const activeTurnId = computeCurrentTurnId(currentEntries)
      if (activeTurnId !== null) {
        nextEntry.turnId = activeTurnId
      }
    }
    setAgentTranscriptEntries(
      agentId,
      trimLiveAgentPaneEntries(agentId, [...currentEntries, nextEntry]),
      turnIds,
    )
  }

  const appendProviderChunkToAgentPane = (
    agentId: string,
    role: TranscriptEntry["role"],
    chunk: string,
    mergeKey?: string,
    sourceText?: string,
  ) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    const normalizedSource = sourceText?.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }

    const currentEntries = currentAgentPaneEntries(agentId).map((entry) => ({ ...entry }))
    const nextEntries = currentEntries.map((entry) => ({ ...entry }))
    if (mergeKey) {
      for (let index = nextEntries.length - 1; index >= 0; index -= 1) {
        const candidate = nextEntries[index]
        if (candidate?.role !== role || candidate.mergeKey !== mergeKey) {
          continue
        }
        if (role === "assistant" || role === "reasoning") {
          candidate.text += normalized
          if (normalizedSource !== undefined) {
            candidate.sourceText = `${candidate.sourceText ?? ""}${normalizedSource}`
          }
        } else {
          candidate.text = normalized
          if (normalizedSource !== undefined) {
            candidate.sourceText = normalizedSource
          }
        }
        commitStreamingAgentPaneEntry(agentId, currentEntries, nextEntries, candidate.id)
        return
      }
    }

    const last = [...nextEntries].reverse().find((entry) => entry.role !== "turn_toggle")
    if (!mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
      last.text += normalized
      commitStreamingAgentPaneEntry(agentId, currentEntries, nextEntries, last.id)
      return
    }

    nextEntries.push({
      id: nextEntries.reduce((max, entry) => Math.max(max, entry.id), 0) + 1,
      role,
      text: normalized,
      ...(computeCurrentTurnId(nextEntries) !== null ? { turnId: computeCurrentTurnId(nextEntries)! } : {}),
      ...(mergeKey ? { mergeKey } : {}),
      ...(normalizedSource !== undefined ? { sourceText: normalizedSource } : {}),
    })
    setAgentTranscriptEntries(agentId, trimLiveAgentPaneEntries(agentId, nextEntries))
  }

  const appendToolUpdateToAgentPane = (agentId: string, chunk: string) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }
    const parsed = parseToolTranscriptUpdate(normalized)
    if (parsed) {
      const toolState = auxiliaryAgentPaneTools(agentId)
      const merged = mergeToolTranscriptUpdate(toolState.get(parsed.id) ?? null, parsed)
      toolState.set(parsed.id, merged)
      appendProviderChunkToAgentPane(agentId, "tool", formatToolTranscriptUpdate(merged), parsed.id, JSON.stringify(merged))
      return
    }
    appendProviderChunkToAgentPane(agentId, "tool", normalized, undefined, normalized)
  }

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

  const mountTranscriptEntry = (entry: TranscriptEntry, requestRender = true) => {
    if (!transcriptScrollbox) {
      return
    }

    if (emptyTranscriptRenderable) {
      transcriptScrollbox.remove(emptyTranscriptRenderable.id)
      emptyTranscriptRenderable.destroyRecursively()
      emptyTranscriptRenderable = undefined
    }

    const renderable = buildTranscriptEntryRenderable(
      renderer,
      entry,
      transcriptSyntax,
      toggleTurn,
      toggleBlob,
      primaryTranscriptSurfaceTone(),
    )
    transcriptRenderables.set(entry.id, renderable)
    transcriptScrollbox.add(renderable.wrapper)
    if (requestRender) {
      requestTranscriptRender()
    }
  }

  const reconcileMountedTranscript = (currentEntries: TranscriptEntry[], nextEntries: TranscriptEntry[]) => {
    if (workflowScreenActive()) {
      rebuildTranscript()
      return
    }
    reconcileMountedTranscriptPane({
      scrollbox: transcriptScrollbox,
      currentEntries,
      nextEntries,
      renderables: transcriptRenderables,
      clampScrollTop,
      rebuild: rebuildTranscript,
      removeEmptyRenderable: () => {
        if (!emptyTranscriptRenderable || !transcriptScrollbox) {
          return
        }
        transcriptScrollbox.remove(emptyTranscriptRenderable.id)
        emptyTranscriptRenderable.destroyRecursively()
        emptyTranscriptRenderable = undefined
      },
      mountEntry: mountTranscriptEntry,
      onScrollTop: (scrollTop) => {
        lastTranscriptScrollTop = scrollTop
      },
    })
  }

  const updateTranscriptEntry = (entryId: number, text: string, sourceText?: string) => {
    const renderable = transcriptRenderables.get(entryId)
    if (!renderable) {
      rebuildTranscript()
      return
    }
    const previousMode = transcriptRenderMode(renderable.entry)
    renderable.entry.text = text
    if (sourceText !== undefined) {
      renderable.entry.sourceText = sourceText
    }
    if (transcriptRenderMode(renderable.entry) !== previousMode) {
      rebuildTranscript()
      return
    }
    renderable.update(renderable.entry)
    requestTranscriptRender()
  }

  const rebuildTranscript = () => {
    logViewDebug("rebuild transcript:start", {
      visible_entries: visibleTranscriptEntries().length,
    })
    if (!transcriptScrollbox) {
      logViewDebug("rebuild transcript:missing scrollbox")
      return
    }

    for (const child of [...transcriptScrollbox.getChildren()]) {
      transcriptScrollbox.remove(child.id)
      child.destroyRecursively()
    }
    transcriptRenderables.clear()
    emptyTranscriptRenderable = undefined

    const visibleEntries = visibleTranscriptEntries()
    if (isAttached() && workflowScreenActive()) {
      emptyTranscriptRenderable = buildWorkflowOutlineRenderable(renderer, {
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
      })
      transcriptScrollbox.add(emptyTranscriptRenderable)
      transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: 0 })
    } else if (visibleEntries.length === 0) {
      emptyTranscriptRenderable = isAttached()
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
        }, waitingRoomTargets(), themeRegistryState())
      transcriptScrollbox.add(emptyTranscriptRenderable)
      if (isAttached()) {
        transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: 0 })
      }
    } else {
      for (const entry of visibleEntries.filter((candidate) => !candidate.historyDeferred)) {
        mountTranscriptEntry(entry, false)
      }
    }

    transcriptScrollbox.requestRender()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
    logViewDebug("rebuild transcript:complete", {
      scroll_height: transcriptScrollbox.scrollHeight,
      scroll_top: transcriptScrollbox.scrollTop,
    })
  }

  const openWorkflowNodeInstructionsEditor = (workflowId: string, nodeId: string, draft: string) => {
    setWorkflowNodeInstructionsEditor({ workflowId, nodeId, draft })
    if (!workflowScreenShowing()) {
      setWorkspaceScreenMode("workflow")
    }
    rebuildTranscript()
    startTimeout(() => {
      workflowNodeInstructionsInput?.focus()
    }, 0)
  }

  const closeWorkflowNodeInstructionsEditor = () => {
    if (!workflowNodeInstructionsEditor()) {
      return
    }
    setWorkflowNodeInstructionsEditor(null)
    workflowNodeInstructionsInput = undefined
    if (workflowScreenShowing()) {
      rebuildTranscript()
    }
    promptInput?.focus()
  }

  const updateWorkflowNodeInstructionsDraft = (draft: string) => {
    const editor = workflowNodeInstructionsEditor()
    if (!editor) {
      return
    }
    setWorkflowNodeInstructionsEditor({ ...editor, draft })
  }

  const getWorkflowNodeInstructionsContext = () => {
    const editor = workflowNodeInstructionsEditor()
    if (!editor) {
      return null
    }
    return { workflowId: editor.workflowId, nodeId: editor.nodeId }
  }

  const getWorkflowNodeInstructionsDraft = () => workflowNodeInstructionsEditor()?.draft ?? ""

  const openWorkflowTerminalPanel = (workflowId: string) => {
    if (workflowNodeInstructionsEditor()) {
      setWorkflowNodeInstructionsEditor(null)
      workflowNodeInstructionsInput = undefined
    }
    setWorkflowInspectorMode("terminal")
    setSelectedWorkflowId(workflowId)
    if (!workflowScreenShowing()) {
      setWorkspaceScreenMode("workflow")
    }
    rebuildTranscript()
  }

  const replaceTranscriptEntries = (
    nextEntries: TranscriptEntry[],
    transcriptAgentId: string | null = visibleTranscriptAgentId(),
  ) => {
    const scrollbox = transcriptScrollbox
    const previousScrollTop = scrollbox?.scrollTop ?? 0
    const previousScrollHeight = scrollbox?.scrollHeight ?? 0
    const previousViewportHeight = scrollbox?.height ?? 0
    const sanitizedEntries = applyTranscriptDisplayState(nextEntries.filter(Boolean), expandedTurnIdsForAgent(transcriptAgentId))
    tools.clear()
    currentTurnId = computeCurrentTurnId(sanitizedEntries)
    nextTurnId = computeNextTurnId(sanitizedEntries)
    setEntries(reconcile(sanitizedEntries))
    setEntryCounter(sanitizedEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    rebuildTranscript()
    mountedTranscriptAgentId = transcriptAgentId
    if (scrollbox && transcriptScrollbox === scrollbox) {
      const nextScrollTop = computeTranscriptRebuildScrollTop({
        previousScrollTop,
        previousScrollHeight,
        nextScrollHeight: scrollbox.scrollHeight,
        viewportHeight: previousViewportHeight,
      })
      scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: nextScrollTop })
      scrollbox.requestRender()
      lastTranscriptScrollTop = scrollbox.scrollTop
    } else {
      lastTranscriptScrollTop = transcriptScrollbox?.scrollTop ?? 0
    }
    syncVisibleTranscriptPreview(transcriptAgentId, sanitizedEntries)
  }

  const mergeHistoryFragments = (older: TranscriptEntry, newer: TranscriptEntry): TranscriptEntry => {
    const sourceText = (older.sourceText ?? older.text) + (newer.sourceText ?? newer.text)
    const mergedBase: TranscriptEntry = {
      ...newer,
      text: newer.text,
      sourceText,
    }
    if (older.historyFragmentStart !== undefined) mergedBase.historyFragmentStart = older.historyFragmentStart
    if (newer.historyFragmentEnd !== undefined) mergedBase.historyFragmentEnd = newer.historyFragmentEnd
    const totalChars = newer.historyTotalChars ?? older.historyTotalChars
    if (totalChars !== undefined) mergedBase.historyTotalChars = totalChars
    if (older.role !== "tool") {
      return applyHistoryDeferral({
        ...mergedBase,
        text: older.text + newer.text,
      })
    }

    const parsed = parseToolTranscriptUpdate(sourceText)
    if (!parsed) {
      const pending: TranscriptEntry = {
        ...mergedBase,
        text: sourceText,
      }
      delete pending.mergeKey
      return {
        ...applyHistoryDeferral(pending),
      }
    }

    const merged = mergeToolTranscriptUpdate(null, parsed)
    return applyHistoryDeferral({
      ...mergedBase,
      text: formatToolTranscriptUpdate(merged),
      mergeKey: parsed.id,
    })
  }

  const stitchPrependedHistory = (olderEntries: TranscriptEntry[], currentEntries: TranscriptEntry[]) => {
    if (olderEntries.length === 0 || currentEntries.length === 0) {
      return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
    }

    const tail = olderEntries.at(-1)
    const head = currentEntries[0]
    if (
      tail?.historyEntryIndex === undefined
      || head?.historyEntryIndex === undefined
      || tail.historyEntryIndex !== head.historyEntryIndex
      || tail.historyFragmentEnd !== head.historyFragmentStart
    ) {
      return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
    }

    return markDeferredHistoryEntries([
      ...olderEntries.slice(0, -1),
      mergeHistoryFragments(tail, head),
      ...currentEntries.slice(1),
    ])
  }

  const prependTranscriptEntries = async (nextEntries: TranscriptEntry[]) => {
    const sanitizedEntries = nextEntries.filter(Boolean)
    if (sanitizedEntries.length === 0) {
      return
    }

    const currentEntries = entries.filter(Boolean)
    const previousScrollHeight = transcriptScrollbox?.scrollHeight ?? 0
    const previousScrollTop = transcriptScrollbox?.scrollTop ?? 0
    const previousViewportHeight = transcriptScrollbox?.height ?? 0
    const nextCombinedEntries = applyTranscriptDisplayState(
      stitchPrependedHistory(sanitizedEntries, currentEntries),
      expandedTurnIdsForAgent(visibleTranscriptAgentId()),
    )
    currentTurnId = computeCurrentTurnId(nextCombinedEntries)
    nextTurnId = computeNextTurnId(nextCombinedEntries)
    setEntries(reconcile(nextCombinedEntries))
    setEntryCounter(nextCombinedEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    rebuildTranscript()
    if (transcriptScrollbox) {
      const scrollbox = transcriptScrollbox
      await historyScrollRestoreController.restorePrependedHistory({
        scrollbox,
        previousScrollTop,
        previousScrollHeight,
        previousViewportHeight,
      })
    }
  }

  const resolveOlderHistoryChunk = async (cursor: SessionHistoryCursor | null) => {
    let nextCursor = cursor
    let resolvedEntries: TranscriptEntry[] = []
    const agentId = visibleTranscriptAgentId()

    while (nextCursor !== null) {
      const historyPage = await getSessionHistory(client, sessionState().id, nextCursor, agentId)
      const hydratedEntries = reindexTranscriptEntries(hydrateTranscriptEntries(historyPage.entries), entryCounter())
      resolvedEntries = resolvedEntries.length === 0
        ? hydratedEntries
        : stitchPrependedHistory(hydratedEntries, resolvedEntries)
      nextCursor = historyPage.next_cursor
      if (resolvedEntries.length === 0 || resolvedEntries[0]?.role === "user" || nextCursor === null) {
        break
      }
    }

    return {
      entries: resolvedEntries,
      nextCursor,
    }
  }

  const clearAgentPaneRuntime = () => {
    agentTranscriptScrollboxes.clear()
    agentTranscriptRenderables.clear()
    agentEmptyTranscriptRenderables.clear()
    agentPaneTools.clear()
    responseAuxiliaryAgentIds.length = 0
  }

  const primeAttachedSessionBinding = async (session: RuntimeSession) => {
    const promptHistoryGeneration = promptHistoryHydrationController.begin()
    const visibleAgentId = selectResponsePaneAgents(
      session.agents,
      session.focused_agent_id,
      splitAgentResponseMode(),
      maxAgentsPerScreen(),
    ).visibleTranscriptAgentId

    if (!visibleAgentId) {
      replaceTranscriptEntries([], null)
      setNextHistoryCursor(null)
      await promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
      return
    }

    const historyPage = await getSessionHistory(client, session.id, null, visibleAgentId)
    await promptHistoryHydrationController.loadAndApply(session.id, promptHistoryGeneration)
    const preparedEntries = reindexTranscriptEntries(
      hydrateTranscriptEntries(historyPage.entries),
      0,
    )

    setAgentPaneEntries((current) => ({
      ...current,
      [visibleAgentId]: preparedEntries.map((entry) => ({ ...entry })),
    }))
    setAgentPanePreview(visibleAgentId, formatTranscriptPreview(preparedEntries))
    replaceTranscriptEntries(
      preparedEntries.map((entry) => ({ ...entry })),
      visibleAgentId,
    )
    setNextHistoryCursor(historyPage.next_cursor)
  }

  const applyDeferredBootstrap = () => {
    const deferred = props.bootstrap.deferred
    if (!deferred) {
      return
    }

    void deferred.providerCatalog?.then((catalog) => {
      setProviderCatalogState(catalog)
      updateSessionChrome()
    }).catch((error) => {
      appLogger?.warn("failed to hydrate provider catalog after bootstrap", {
        error: formatError(error),
      })
    })

    void deferred.providerCommandCatalogs?.then((catalogs) => {
      setProviderCommandCatalogState(catalogs)
    }).catch((error) => {
      appLogger?.warn("failed to hydrate provider command catalog after bootstrap", {
        error: formatError(error),
      })
    })

    void deferred.attachedHistory?.then(async (history) => {
      if (attachmentState()?.session_id !== history.sessionId) {
        return
      }
      setPromptHistoryEntries(history.promptHistoryEntries)
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(null)
      if (!history.visibleAgentId) {
        setNextHistoryCursor(history.nextHistoryCursor)
        return
      }
      const visibleAgentId = history.visibleAgentId
      const preparedEntries = history.historyEntries.map((entry) => ({ ...entry }))
      if (preparedEntries.length === 0) {
        setNextHistoryCursor(history.nextHistoryCursor)
        return
      }
      setAgentPaneEntries((current) => ({
        ...current,
        [visibleAgentId]: preparedEntries.map((entry) => ({ ...entry })),
      }))
      setAgentPanePreview(visibleAgentId, formatTranscriptPreview(preparedEntries))
      if (entries.filter(Boolean).length === 0) {
        replaceTranscriptEntries(preparedEntries, visibleAgentId)
      } else {
        await prependTranscriptEntries(reindexTranscriptEntries(preparedEntries, entryCounter()))
      }
      setNextHistoryCursor(history.nextHistoryCursor)
    }).catch((error) => {
      appLogger?.warn("failed to hydrate attached history after bootstrap", {
        error: formatError(error),
      })
    })
  }

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

  const submitPrompt = async () => {
    if (!promptInput) {
      return
    }

    ensureBackgroundPollersStarted()

    const rawPrompt = promptInput.plainText
    const trimmed = rawPrompt.trim()
    if (!trimmed && pendingAttachments().length === 0) {
      promptTextController.clear()
      return
    }
    if (workflowScreenShowing() && isWorkspaceShellCommand(rawPrompt)) {
      try {
        await submitWorkspaceShellCommand(rawPrompt)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptTextController.clear()
      }
      return
    }
    if (workflowNodeInstructionsEditor() && !trimmed.startsWith("/")) {
      flashFooter("instructions editor is open; type in the I/O panel and use /workflow node instructions save", "info")
      promptTextController.clear()
      return
    }
    const allowSlashCommandSubmission = !workflowScreenShowing() || isWorkflowCommandInput(rawPrompt)
    const providerNamespaceCommand = parseProviderNamespaceCommand(
      rawPrompt,
      focusedBackendProvider(),
    )
    const handledCommand = await slashCommandSubmitController.submit(rawPrompt, {
      allowSlashCommandSubmission,
      trimmedPrompt: trimmed,
    })
    if (handledCommand) {
      return
    }
    if (providerNamespaceCommand) {
      const submitDecision = validateProviderNamespaceSubmit({
        command: providerNamespaceCommand,
        focusedProvider: focusedBackendProvider(),
        workflowScreenShowing: workflowScreenShowing(),
        pendingAttachmentCount: pendingAttachments().length,
      })
      if (!submitDecision.ok) {
        flashFooter(submitDecision.message, "error")
        return
      }

      let submissionUi: SubmittedPromptUiSnapshot | null = null
      try {
        await waitForPendingAgentFocusTransition()
        const targetAgentId = focusedAgentId()
        activeToolLabels.clear()
        setProviderActivityLabel(null)
        setActiveStatusLabel(null)
        const attachment = attachmentState()
        if (!attachment) {
          flashFooter("No session attached.", "error")
          promptTextController.clear()
          return
        }
        submissionUi = beginSubmittedPromptUi(rawPrompt)
        appendUserPrompt(renderPromptTranscript(providerNamespaceCommand.raw), targetAgentId)
        const forwardedPrompt = `${submitDecision.forwardedCommand}\n`
        const submission = await submitPromptWithRecovery(
          client,
          sessionState().id,
          attachment.id,
          targetAgentId,
          forwardedPrompt,
          [],
          options,
          appLogger,
        )
        const payload = submission.payload
        const submittedTargetAgentId = submission.targetAgentId ?? targetAgentId
        applySessionState(payload.session)
        setStreamingAgentId(submittedTargetAgentId)
        setWorking(true)
        updateSessionChrome()
        recordPromptAreaHistoryEntry(sessionState().id, rawPrompt)
        clearCommandCenter()
      } catch (error) {
        appLogger?.error("provider namespace command failed", {
          command: providerNamespaceCommand.raw,
          error: formatError(error),
        })
        restoreFailedPromptUi(submissionUi)
        clearAgentBusy(submittingAgentId)
        submittingAgentId = null
        setSubmitting(false)
        setWorking(false)
        setFatalError(formatError(error))
        updateSessionChrome()
      }
      return
    }
    if (!isAttached()) {
      flashFooter(SESSION_NEW_ERROR_HINT, "error")
      promptTextController.clear()
      return
    }

    if (workflowScreenShowing()) {
      await workflowPromptSubmitController.submit(rawPrompt)
      return
    }

    const prompt = formatPromptSubmissionBody(rawPrompt)
    const rawAttachments = pendingPromptAttachmentsToParts(pendingAttachments())
    let submissionUi: SubmittedPromptUiSnapshot | null = null
    try {
      await waitForPendingAgentFocusTransition()
      const targetAgentId = focusedAgentId()
      appLogger?.info("submitting prompt", {
        chars: prompt.length,
        attachments: rawAttachments.length,
      })
      activeToolLabels.clear()
      setProviderActivityLabel(null)
      setActiveStatusLabel(null)
      const attachment = attachmentState()
      if (!attachment) {
        flashFooter("No session attached.", "error")
        promptTextController.clear()
        return
      }
      const attachments = await preparePromptAttachmentsForSubmit(rawAttachments, {
        inlineLocalFiles: Boolean(options.relayUrl) || promptAttachmentTransferIsForced(),
      })
      submissionUi = beginSubmittedPromptUi(rawPrompt)
      appendUserPrompt(renderPromptTranscript(prompt), targetAgentId)
      const submission = await submitPromptWithRecovery(
        client,
        sessionState().id,
        attachment.id,
        targetAgentId,
        prompt,
        attachments,
        options,
        appLogger,
      )
      const payload = submission.payload
      const submittedTargetAgentId = submission.targetAgentId ?? targetAgentId
      applySessionState(payload.session)
      setStreamingAgentId(submittedTargetAgentId)
      setWorking(true)
      updateSessionChrome()
      const outcomeName = submission.outcomeName
      appLogger?.info("prompt submitted", {
        outcome: outcomeName,
        active_prompt_id: payload.session.active_prompt?.id ?? null,
        queued_prompts: payload.session.queued_prompts.length,
      })
      setStatusLine(
        formatPromptSubmissionStatusLine({
          outcomeName,
          activePromptId: payload.session.active_prompt?.id ?? null,
        }),
      )
      updateSessionChrome()
      recordPromptAreaHistoryEntry(sessionState().id, rawPrompt)
    } catch (error) {
      appLogger?.error("prompt submission failed", {
        error: formatError(error),
      })
      restoreFailedPromptUi(submissionUi)
      clearAgentBusy(submittingAgentId)
      submittingAgentId = null
      setSubmitting(false)
      setWorking(false)
      setFatalError(formatError(error))
      updateSessionChrome()
    }
  }

  const requestPromptStop = async () => {
    await promptStopController.request()
  }

  const submitFocusedInteractionChoice = async (choiceIndex?: number) => {
    const interaction = focusedAgentInteraction()
    if (!interaction || !isAttached()) {
      return false
    }
    const submitDecision = resolveInteractionChoiceSubmission({
      interaction,
      requestedIndex: choiceIndex,
      selectedIndex: interactionChoiceSelection.get(interaction.id),
      customReply: interactionCustomReplies.get(interaction.id) ?? "",
    })
    if (submitDecision.action === "unavailable") {
      return false
    }
    if (submitDecision.action === "edit_custom") {
      interactionCustomEditing.add(interaction.id)
      renderAgentInteractions()
      applyResponseLayout()
      return true
    }
    interactionChoiceSelection.set(interaction.id, submitDecision.selectedIndex)
    try {
      const session = await respondToInteraction(
        client,
        sessionState().id,
        interaction.id,
        submitDecision.choiceId,
        submitDecision.customReply,
      )
      applySessionState(session)
      interactionCustomReplies.delete(interaction.id)
      interactionCustomEditing.delete(interaction.id)
      flashFooter("interaction answered", "info")
      return true
    } catch (error) {
      flashFooter(formatError(error), "error")
      return true
    }
  }

  const cycleFocusedInteractionChoice = (delta: number) => {
    const interaction = focusedAgentInteraction()
    if (!interaction) {
      return false
    }
    const currentIndex = interactionChoiceSelection.get(interaction.id) ?? 0
    const nextIndex = nextInteractionChoiceIndex({ interaction, currentIndex, delta })
    if (nextIndex === null) {
      return false
    }
    interactionChoiceSelection.set(interaction.id, nextIndex)
    if (interaction.custom_choice && nextIndex !== interactionCustomChoiceIndex(interaction)) {
      interactionCustomEditing.delete(interaction.id)
    }
    renderAgentInteractions()
    applyResponseLayout()
    return true
  }

  const handleFocusedInteractionKey = (event: {
    name: string
    eventType?: string
    ctrl?: boolean
    meta?: boolean
    alt?: boolean
    preventDefault?: () => void
    stopPropagation?: () => void
  }) => {
    const interaction = focusedAgentInteraction()
    if (!interaction || event.eventType === "release") {
      return false
    }
    const keyAction = resolveInteractionChoiceKeyAction({
      interaction,
      event,
      selectedIndex: interactionChoiceSelection.get(interaction.id) ?? 0,
      customEditing: interactionCustomEditing.has(interaction.id),
      customReply: interactionCustomReplies.get(interaction.id) ?? "",
    })
    if (keyAction.action === "ignore") {
      return false
    }
    if (keyAction.consumeEvent) {
      event.preventDefault?.()
      event.stopPropagation?.()
    }
    if (keyAction.action === "handled") {
      return true
    }
    if (keyAction.action === "cancel_custom_edit") {
      interactionCustomEditing.delete(interaction.id)
      renderAgentInteractions()
      applyResponseLayout()
      return true
    }
    if (keyAction.action === "delete_custom_reply") {
      interactionCustomReplies.set(interaction.id, deleteInteractionCustomReply(interactionCustomReplies.get(interaction.id) ?? ""))
      renderAgentInteractions()
      applyResponseLayout()
      return true
    }
    if (keyAction.action === "append_custom_reply") {
      const current = interactionCustomReplies.get(interaction.id) ?? ""
      interactionCustomReplies.set(interaction.id, appendInteractionCustomReply({
        current,
        input: keyAction.input,
        maxLength: interaction.custom_choice?.max_length,
      }))
      renderAgentInteractions()
      applyResponseLayout()
      return true
    }
    if (keyAction.action === "cycle") {
      return cycleFocusedInteractionChoice(keyAction.delta)
    }
    if (keyAction.action === "begin_custom_edit") {
      interactionChoiceSelection.set(interaction.id, keyAction.selectedIndex)
      interactionCustomEditing.add(interaction.id)
      renderAgentInteractions()
      applyResponseLayout()
      return true
    }
    if (keyAction.action === "submit") {
      void submitFocusedInteractionChoice(keyAction.choiceIndex)
      return true
    }
    return false
  }

  useKeyboard((event) => {
    if (handleHotkeysToggleShortcut("keyboard", event)) {
      return
    }
    if (dialogOverlayOpen() && event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      closeActiveDialogOverlay()
      return
    }
    if (event.ctrl && event.name === "e") {
      event.preventDefault()
      event.stopPropagation()
      void requestExit()
      return
    }
    if (event.ctrl && event.name === "c") {
      event.preventDefault()
      event.stopPropagation()
      void (activePrompt() ? requestPromptStop() : requestExit())
      return
    }
    if (dialogOverlayOpen()) {
      event.preventDefault()
      event.stopPropagation()
    }
  })

  const handleSigint = () => {
    void (activePrompt() ? requestPromptStop() : requestExit())
  }
  const handlePromptHistoryKey = (event: {
    name: string
    eventType?: string
    ctrl?: boolean
    meta?: boolean
    alt?: boolean
    shift?: boolean
    preventDefault?: () => void
    stopPropagation?: () => void
  }) => {
    const direction = resolvePromptHistoryKeyNavigation({
      attached: isAttached(),
      promptFocused: Boolean(promptInput?.focused),
      commandCenterOpen: commandCenterOpen(),
      keyName: event.name,
      currentText: promptTextController.currentText(),
      cursorOffset: promptTextController.cursorOffset(),
      eventType: event.eventType,
      ctrl: event.ctrl,
      meta: event.meta,
      alt: event.alt,
      shift: event.shift,
      navigationIndex: promptHistoryIndex(),
      navigationDraft: promptHistoryDraft(),
    })
    if (!direction) {
      return false
    }
    const handled = navigatePromptHistoryInput(direction)
    if (handled) {
      event.preventDefault?.()
      event.stopPropagation?.()
    }
    return handled
  }
  const shouldNavigatePromptTurns = (event: { name: string; eventType: string; shift?: boolean }) => {
    return promptTurnNavigationDirectionForKey({
      attached: isAttached(),
      keyName: event.name,
      eventType: event.eventType,
      shift: event.shift,
      promptText: promptInput ? promptTextController.currentText() : undefined,
    }) !== null
  }
  const navigatePromptTurns = (direction: "previous" | "next") => {
    if (!transcriptScrollbox) {
      return
    }
    const promptOffsets = visibleTranscriptEntries()
      .filter((entry) => entry.role === "user")
      .map((entry) => transcriptRenderables.get(entry.id)?.wrapper.y ?? null)
      .filter((offset): offset is number => offset !== null)
      .sort((left, right) => left - right)
    const target = findTurnPromptScrollTarget(promptOffsets, transcriptScrollbox.scrollTop, direction)
    if (target === null || target === undefined) {
      return
    }
    transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: target })
    transcriptScrollbox.requestRender()
    lastTranscriptScrollTop = transcriptScrollbox.scrollTop
  }
  const handleStdinData = (chunk: Buffer | string) => {
    const event = parseKeypress(chunk, { useKittyKeyboard: true })
    if (!event) {
      return
    }
    if (event.eventType !== "release" && dialogOverlayOpen() && event.name === "escape") {
      closeActiveDialogOverlay()
      return
    }
    if (handleSessionBrowserKey(event)) {
      return
    }
    if (event.eventType !== "release" && event.ctrl && event.name === "e") {
      void requestExit()
      return
    }
    if (handleFocusedInteractionKey(event)) {
      return
    }
    if (promptInput?.focused && commandCenterOpen()) {
      if (event.eventType !== "release" && event.name === "escape") {
        clearCommandCenter()
      }
      return
    }
    if (event.eventType !== "release" && event.ctrl && event.name === "p") {
      if (dialogOverlayOpen()) {
        return
      }
      toggleWorkspaceScreen()
      return
    }
    if (shouldCycleFocusOnTabEvent(event, {
      attached: isAttached(),
      hotkeysOpen: dialogOverlayOpen(),
      promptFocused: Boolean(promptInput?.focused),
      commandCenterOpen: commandCenterOpen(),
      commandCenterQuery: commandCenterController.query(),
    })) {
      if (workflowScreenActive()) {
        cycleWorkflowCanvasNode()
      } else {
        void handleCycleAgentFocus()
      }
      return
    }
    if (event.eventType !== "release" && event.meta && event.name === "c" && copyPromptSelection()) {
      return
    }
    if (event?.ctrl && event.name === "c") {
      void (activePrompt() ? requestPromptStop() : requestExit())
      return
    }
    if (dialogOverlayOpen()) {
      return
    }
    if (event.eventType !== "release" && promptInput?.focused) {
      if (event.name === "backspace" && removePromptAttachmentsForEdit("backspace")) {
        return
      }
      if (event.name === "delete" && removePromptAttachmentsForEdit("delete")) {
        return
      }
    }
    if (event.eventType !== "release" && event.name === "backspace" && isAttached() && !promptTextController.currentText() && pendingAttachments().length > 0) {
      removeLastPendingPromptAttachment()
      return
    }
    if (shouldNavigatePromptTurns(event)) {
      navigatePromptTurns(event.name === "up" ? "previous" : "next")
      return
    }
    if (shouldHandleWaitingRoomKeyEvent(event, {
      attached: isAttached(),
      hotkeysOpen: dialogOverlayOpen(),
      promptFocused: Boolean(promptInput?.focused),
      commandCenterOpen: commandCenterOpen(),
      commandCenterQuery: commandCenterController.query(),
    })) {
      const keyNavigation = deriveWaitingRoomKeyNavigationDecision({
        event,
        state: waitingRoomState(),
        sessions: availableSessions(),
        catalog: providerCatalogState(),
        remote: {
          relay: relayStatusState(),
          machines: remoteMachinesState(),
          kernels: remoteKernelsState(),
          terminals: terminalsState(),
          slices: slicesState(),
        },
        themeRegistry: themeRegistryState(),
      })
      if (keyNavigation.action === "navigate") {
        reconcileWaitingRoom(keyNavigation.nextState)
        return
      }
      if (keyNavigation.action === "release") {
        setWaitingRoomState(keyNavigation.nextState)
        rebuildTranscript()
        return
      }
      const sessionLifecycleAction = waitingRoomSessionLifecycleActionForEvent({
        event,
        promptFocused: Boolean(promptInput?.focused),
      })
      if (sessionLifecycleAction) {
        void applyWaitingRoomSessionLifecycleAction(sessionLifecycleAction)
        return
      }
      if (event.eventType !== "release" && (event.name === "return" || event.name === "enter")) {
        void activateWaitingRoom()
      }
    }
  }

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

  const automationSocketPath = options.automationSocket
  let automationServer: CliAutomationServer | null = null
  if (automationSocketPath) {
    void startCliAutomationServer({
      socketPath: automationSocketPath,
      handleRequest: handleAutomationRequest,
      formatError,
      onListening: (socketPath) => {
        appLogger?.info("cli automation socket listening", { socket_path: socketPath })
      },
    })
      .then((server) => {
        automationServer = server
      })
      .catch((error) => {
        appLogger?.error("failed to start cli automation socket", {
          socket_path: automationSocketPath,
          error: formatError(error),
        })
        flashFooter(`automation socket failed: ${formatError(error)}`, "error")
      })
  }
  process.on("SIGINT", handleSigint)
  process.stdin.on("data", handleStdinData)
  onCleanup(() => {
    process.off("SIGINT", handleSigint)
    process.stdin.off("data", handleStdinData)
    if (automationServer && automationSocketPath) {
      stopCliAutomationServer(automationServer, automationSocketPath)
      automationServer = null
    }
    terminalOutputRecordQueue.clearTimer()
  })

  let pollersStarted = false
  const onResize = () => {
    if (isAttached()) {
      void maybeResize(client, sessionState().id)
    }
  }

  const markPollerDegraded = (operation: string, message: string) => {
    const wasHealthy = degradedPollers.size === 0
    degradedPollers.add(operation)
    setDaemonDisconnected(true)
    appLogger?.warn("poller entered degraded mode", {
      operation,
      degraded_pollers: [...degradedPollers],
    })
    setStatusLine(message)
    updateSessionChrome()
    if (wasHealthy) {
      appendNotice(message, "warning")
    }
  }

  const markPollerRecovered = (operation: string, failureCount: number) => {
    if (failureCount === 0) {
      return
    }
    const wasDegraded = degradedPollers.delete(operation)
    if (wasDegraded) {
      appLogger?.info("poller recovered", {
        operation,
        degraded_pollers: [...degradedPollers],
        prior_failures: failureCount,
      })
    }
    if (wasDegraded && degradedPollers.size === 0) {
      setDaemonDisconnected(false)
      setStatusLine(DEFAULT_CONNECTED_STATUS)
      updateSessionChrome()
      appendNotice("Reconnected to the Arroba daemon.")
    }
  }

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

  const ensureBackgroundPollersStarted = () => {
    if (pollersStarted) {
      logViewDebug("ensure pollers:already started")
      return
    }
    if (!promptInput || !transcriptScrollbox) {
      logViewDebug("ensure pollers:missing refs", {
        has_prompt_input: Boolean(promptInput),
      })
      return
    }
    pollersStarted = true
    logViewDebug("ensure pollers:starting")
    rebuildTranscript()
    syncPromptPlaceholder()
    if (isAttached()) {
      promptInput.focus()
    } else {
      promptInput.blur()
    }
    lastTranscriptScrollTop = transcriptScrollbox.scrollTop
    process.stdout.on("resize", onResize)
    if (supportsKernelEventStream) {
      appLogger?.info("starting kernel event stream")
      void syncKernelEventSubscription()
    } else {
      appLogger?.info("starting background pollers")
      void pollOutput()
      void pollNotices()
      void pollSessionState()
    }
    startConnectionWatchdog()
  }

  onCleanup(() => {
    closing = true
    connectionHealthWatchdogController.stop()
    process.stdout.off("resize", onResize)
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

  const transcriptScrollMonitor = startInterval(() => {
    transcriptHistoryAutoloadController.monitorScroll()
  }, 75)

  onCleanup(() => {
    clearInterval(transcriptScrollMonitor)
  })

  const workingAnimation = startInterval(() => {
    setWorkingAnimationFrame((value) => value + 1)
    if (sessionStatusMode() === "working") {
      updateSessionChrome()
    }
    if (splitAgentResponseMode()) {
      renderSplitPaneFooters()
    }
  }, 120)

  onCleanup(() => {
    clearInterval(workingAnimation)
  })

  const waitingRoomAnimation = startInterval(() => {
    const state = waitingRoomState()
    const nextIntroStep = nextWaitingRoomIntroStep(isAttached(), state.introStep)
    if (nextIntroStep === null) {
      return
    }
    setWaitingRoomState({
      ...state,
      introStep: nextIntroStep,
    })
    rebuildTranscript()
  }, 90)

  onCleanup(() => {
    clearInterval(waitingRoomAnimation)
  })

  const relayMachineRefresh = startInterval(() => {
    void refreshWaitingRoomData()
  }, 2_500)

  onCleanup(() => {
    clearInterval(relayMachineRefresh)
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
      onResponseSurfaceMouseUp={(event) => {
        if (event.button !== MouseButton.LEFT) {
          return
        }
        startTimeout(() => {
          copySelection()
          retainPromptFocus()
        }, 0)
      }}
      onFooterMouseUp={(event) => {
        if (event.button !== MouseButton.LEFT) {
          return
        }
        startTimeout(() => {
          copySelection()
          retainPromptFocus()
        }, 0)
      }}
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
      onPromptKeyDown={(event) => {
        if (handleFocusedInteractionKey(event)) {
          return
        }
        if (handleCommandCenterKey(event)) {
          return
        }
        if (handlePromptHistoryKey(event)) {
          return
        }
        handleHotkeysToggleShortcut("textarea", event)
      }}
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
