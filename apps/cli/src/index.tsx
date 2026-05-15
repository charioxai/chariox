import path from "node:path"
import process from "node:process"
import { execFile, spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { unlink } from "node:fs/promises"
import { homedir } from "node:os"
import { pathToFileURL } from "node:url"
import { createConnection, createServer, type Server as NetServer, type Socket as NetSocket } from "node:net"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"

import { BoxRenderable, MouseButton, RGBA, ScrollBoxRenderable, SyntaxStyle, TextAttributes, TextRenderable, addDefaultParsers, parseKeypress, type KeyBinding, type Renderable, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js"
import { createStore, reconcile } from "solid-js/store"

import {
  normalizeRuntimeSession,
} from "./cli-types.js"
import type {
  AgentInstance,
  BootstrapState,
  CaptureScreenshotResult,
  CliOptions,
  PromptAttachmentPart,
  PromptInputHistoryPage,
  PromptSubmittedPayload,
  RuntimeAttachment,
  RuntimeInteraction,
  RuntimeNoticeRecord,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryCursor,
  SessionHistoryEntry,
  SessionHistoryPage,
  SessionHistoryPageEntry,
  SliceRecord,
  StoredTransferArtifact,
  TerminalOutputRecord,
  TranscriptEntry,
  WaitingRoomPublicSessionSummary,
  WaitingRoomPublicSnapshot,
  WorkflowDefinition,
  WorkspaceLinkDefinition,
} from "./cli-types.js"
import {
  createCommandActionHandlers,
} from "./command-actions.js"
import {
  computeTranscriptRebuildScrollTop,
  evaluateTranscriptScrollMonitor,
  nextWaitingRoomIntroStep,
  shouldLoadShortViewportHistory,
} from "./background-effects.js"
import {
  executeSlashCommand,
  parseSlashCommand,
  shouldClearCommandCenterForSlashCommand,
  type ParsedSlashCommand,
} from "./commands.js"
import {
  buildCommandCenterItems,
  nextCommandCenterIndex,
  shouldSubmitExactCommandCenterMatch,
  type CommandCenterItem,
} from "./command-center.js"
import { refreshAgentPaneState, selectCurrentAgentPaneEntries, trimAgentPaneEntries } from "./agent-pane-state.js"
import { parseProviderNamespaceCommand } from "./provider-command-catalog.js"
import { copyTextToClipboard } from "./clipboard.js"
import { HOTKEY_TOGGLE_LABEL, matchHotkeysToggleEvent, shouldCycleFocusOnTabEvent, shouldHandleWaitingRoomKeyEvent } from "./hotkeys.js"
import { clampScrollTop, computePrependedHistoryScrollTop, findTurnPromptScrollTarget } from "./history-viewport.js"
import { createDefaultShellContext, type ShellContext } from "@arroba/kernel-client/shell-core"
import { executeShellLine } from "@arroba/kernel-client/shell-script"
import { KernelEvent, LocalIpcClient } from "./ipc.js"
import { createKernelEventController } from "./kernel-event-controller.js"
import { runClaudeNativeTui } from "./native-tui/claude.js"
import { runCodexNativeTui } from "./native-tui/codex.js"
import { runOpenCodeNativeTui } from "./native-tui/opencode.js"
import {
  attachWorkspaceLinkRequest,
  cancelActivePromptRequest,
  captureScreenshotRequest,
  createWorkspaceLinkRequest,
  cycleAgentFocusRequest,
  destroyAgentRequest,
  detachFromSessionRequest,
  detachWorkspaceLinkRequest,
  endSessionRequest,
  focusAgentRequest,
  getPromptInputHistoryRequest,
  getSessionHistoryRequest,
  getSessionStateRequest,
  getWaitingRoomPublicSnapshotRequest,
  listWorkspaceLinksRequest,
  acceptCloudSessionInviteRequest,
  createCloudSessionInviteRequest,
  createSessionInviteRequest,
  joinSessionInviteRequest,
  listCloudCollaboratorsRequest,
  listCloudSessionMembersRequest,
  pollRuntimeNoticesRequest,
  respondToInteractionRequest,
  recordPromptInputHistoryRequest,
  pumpTerminalOutputRequest,
  resizeTerminalRequest,
  showWorkspaceLinkRequest,
  spawnAgentRequest,
  storeTransferredFileRequest,
  submitPromptRequest,
} from "./ipc-requests.js"
import { expectVariant, firstVariantName } from "./ipc-response.js"
import {
  getUserConfig,
  getUserConfigSchema,
  setUserConfigValue,
  unsetUserConfigValue,
} from "./config-api.js"
import {
  aliasAgent,
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
import { createProcessLogger, type ArrobaLogger } from "./logging.js"
import { runLogViewer } from "./logs.js"
import { evaluateConnectionHealth, runPollingLoop } from "./polling-effects.js"
import {
  bootstrapCloudRelayProfile,
} from "./cloud-relay.js"
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
import {
  extractDroppedPromptAttachments,
  parsePromptAttachmentCommand,
  resolvePromptAttachmentEdit,
  type ParsedPromptAttachment,
  type PromptAttachmentKind,
} from "./prompt-attachments.js"
import {
  preparePromptAttachmentsForSubmit,
  promptAttachmentTransferIsForced,
} from "./prompt-attachment-transfer.js"
import type { PromptMetaPart, PromptMetaTone } from "./prompt-meta.js"
import {
  backendProviderLabel,
  type BackendProviderId,
  fallbackProviderCatalog,
  isBackendProviderId,
  normalizeBackendProviderId,
  selectConfiguredModel,
  selectConfiguredVariant,
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
  providerRunUsesNativeTui,
  sameProviderRun,
  startProviderLogin,
  teardownProviderProcesses,
  tryGetProviderRun,
  updateSessionConfig,
} from "./provider-api.js"
import {
  applyHistoryDeferral,
  hydrateTranscriptEntries,
  markDeferredHistoryEntries,
} from "./transcript-history.js"
import { buildPaneGridModel, type PaneGridTone } from "./response-pane-grid.js"
import {
  responsePaneRowSlots,
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "./response-panes.js"
import {
  extractPromptHistoryEntries,
  extractPromptInputHistoryEntries,
  isProgrammaticPromptContentEcho,
  navigatePromptHistory,
  promptHistoryDirectionForKey,
  pushPromptHistoryEntry,
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
  type StatusBadgePart,
} from "./session-chrome-state.js"
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
import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
} from "./transcript-display.js"
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
  sessionBrowserSortTime,
  sessionBrowserStatus,
  sessionBrowserTimestamp,
  sessionBrowserTitle,
  type SessionListEntry,
} from "./sessions.js"
import {
  aliasSession,
  archiveSessionById,
  attachToSession,
  createSession,
  deleteSessionByRef,
  getSessionState,
  listSessions,
  resolveSession,
} from "./session-api.js"
import {
  agentPaneStatusBadge,
  formatSplitPaneFooterParts,
  reflectedDistance,
  type StatusBadgeTone,
} from "./split-pane-footer.js"
import { syncAuxiliaryPane } from "./response-layout-render.js"
import { createRenderScheduler } from "./render-scheduler.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { applyTheme, createTranscriptSyntaxStyle, EmptyBorder, setThemeRegistry, SplitBorder, theme } from "./theme.js"
import { DEFAULT_THEME_REGISTRY, loadThemeRegistry } from "./theme-registry.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomDeleteDecision,
  deriveWaitingRoomModelSelectionDecision,
  deriveWaitingRoomSessionLifecycleDecision,
  deriveWaitingRoomStateUpdate,
  deriveWaitingRoomVariantSelectionDecision,
  type WaitingRoomDeleteDecision,
  type WaitingRoomSessionLifecycleDecision,
  type WaitingRoomSessionLifecycleAction,
} from "./waiting-room-controller.js"
import {
  createWaitingRoomState,
  cycleWaitingRoomValue,
  moveWaitingRoomFocus,
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteKernelIsAttachable,
  waitingRoomRows,
  type WaitingRoomFocus,
  type WaitingRoomState,
} from "./waiting-room.js"
import {
  primeWaitingRoomWorktreeInventory,
} from "./waiting-room-worktrees.js"
import {
  resolveWorkspaceVisibleAgents,
  resolveWorkspaceVisibleTranscriptAgentId,
  type WorkspaceScreenMode,
} from "./workspace-screen.js"
import {
  appendWorkspaceShellEntry,
  isWorkspaceShellCommand,
  renderWorkspaceShellTranscript,
  workspaceShellCommandText,
  type WorkspaceShellEntry,
} from "./workspace-shell.js"
import { createWorkflowController, deriveWorkflowSelectionState } from "./workflow-controller.js"
import {
  deriveWorkflowPromptState,
  formatWorkflowPromptPlaceholder,
  isWorkflowCommandInput,
  resolveActiveWorkflowRun,
} from "./workflow-prompt-state.js"
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
  formatPairingExpiry,
  formatTerminalTypeLabel,
  getRelayStatus,
  issueKernelCloudRelayClientToken,
  logoutCloudRelay,
  pairKernelCloudRelayClient,
  pairKernelCloudRelayMachine,
  pollCloudRelayLogin,
  renderTerminalPairingQr,
  startCloudRelayLogin,
  wrapPairingLink,
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

const execFileAsync = promisify(execFile)

const PROMPT_KEYBINDINGS = [
  { name: "return", action: "submit" },
  { name: "return", shift: true, action: "newline" },
  { name: "return", meta: true, action: "newline" },
] satisfies KeyBinding[]

const BOOTSTRAP_HISTORY_MAX_CHARS = 100_000
const HISTORY_PAGE_ROUND_COUNT = 1
const LIVE_TRANSCRIPT_LIMIT = 400
const LIVE_TRANSCRIPT_MAX_CHARS = 250_000
const WAITING_ROOM_SESSION_ACTION_CONFIRM_MS = 4_000
const STREAM_BATCH_WINDOW_MS = 48
const CHROME_UPDATE_THROTTLE_MS = 48
const TURN_COMPLETION_QUIET_MS = 1_500
const COMMAND_CENTER_OVERLAY_FOOTPRINT = 3
const ATTACHED_PROMPT_PLACEHOLDER = "Write your next prompt here"
const HOTKEY_DIALOG_WIDTH = 72

type HotkeyItem = {
  keys: string
  description: string
}

type PendingWaitingRoomSessionAction = {
  action: WaitingRoomSessionLifecycleAction
  targetKind: "session" | "sessions" | "machine" | "kernel"
  targetId: string
  expiresAtMs: number
}

type RemoteMachineView = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name: string
  trust_status: "approved" | "pending" | "forgotten"
  online: boolean
  pending: boolean
  kernel_count: number
  available_providers?: string[]
}

type RemoteKernelView = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  kernel_alias?: string | null
  relay_alias?: string | null
  available_providers?: string[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

type HotkeySection = {
  title: string
  items: HotkeyItem[]
}

const GLOBAL_HOTKEYS: HotkeyItem[] = [
  { keys: HOTKEY_TOGGLE_LABEL, description: "Show or hide this hotkey list." },
  { keys: "Ctrl+E", description: "Exit the CLI with the same behavior as /exit." },
  { keys: "Ctrl+C", description: "Stop the active agent; if idle, exit the CLI." },
]

const SESSION_HOTKEYS: HotkeyItem[] = [
  { keys: "Enter", description: "Submit the current prompt." },
  { keys: "Shift+Enter", description: "Insert a newline in the prompt." },
  { keys: "Tab", description: "Cycle focus to the next agent or workflow node." },
  { keys: "Ctrl+P", description: "Toggle between the agent screens and workflow outline." },
  { keys: "Up / Down", description: "Browse submitted prompts in the prompt area." },
  { keys: "Shift+Up / Shift+Down", description: "Jump between user turns when the prompt is empty." },
  { keys: "Backspace / Delete", description: "Remove pending attachment tokens from the prompt." },
]

const WAITING_ROOM_HOTKEYS: HotkeyItem[] = [
  { keys: "Arrow keys", description: "Move through options and browse the visible session preview." },
  { keys: "Enter", description: "Create, attach, or open the full session list from Join Existing Session." },
  { keys: "A", description: "Archive the selected session, or all sessions from Join Existing Session, after confirmation." },
  { keys: "D / Delete", description: "Delete the selected session, all sessions, or inactive remote inventory after confirmation." },
]

const promptTokenStyle = SyntaxStyle.create()
const promptTokenStyleIds = {
  image: promptTokenStyle.registerStyle("prompt-token-image", {
    fg: RGBA.fromHex("#1f1400"),
    bg: RGBA.fromHex("#f0d77d"),
    bold: true,
  }),
  pdf: promptTokenStyle.registerStyle("prompt-token-pdf", {
    fg: RGBA.fromHex("#09182b"),
    bg: RGBA.fromHex("#8cc0ff"),
    bold: true,
  }),
  file: promptTokenStyle.registerStyle("prompt-token-file", {
    fg: RGBA.fromHex("#0d1f13"),
    bg: RGBA.fromHex("#8fd8a8"),
    bold: true,
  }),
}

type PromptQueueItem = {
  id: string
  source_attachment_id: string
  target_agent_id?: string | null
  prompt: string
  status: string
}

type PendingPromptAttachment = {
  id: string
  url: string
  mime: string
  filename: string
  kind: PromptAttachmentKind
  token: string
}

type FooterFlash = {
  message: string
  tone: "info" | "error"
}

type SplitPaneFooterTextGroup = {
  agentText?: TextRenderable
  agentDividerText?: TextRenderable
  providerText?: TextRenderable
  providerDividerText?: TextRenderable
  modelText?: TextRenderable
  modelDividerText?: TextRenderable
  variantText?: TextRenderable
  variantDividerText?: TextRenderable
  modeText?: TextRenderable
  modeDividerText?: TextRenderable
  permissionText?: TextRenderable
}

const DEBUG_LOGS_ENABLED = (process.env.ARROBA_LOG_LEVEL ?? "").toLowerCase() === "debug"
const OPEN_CONSOLE_ON_ERROR = process.env.ARROBA_OPEN_CONSOLE_ON_ERROR === "1"
let processLogger: ArrobaLogger | null = null
let transcriptParsersRegistered = false

type CliAutomationRequest = {
  id?: unknown
  action?: unknown
  attachments?: unknown
  command?: unknown
  prompt?: unknown
  screen?: unknown
  choiceIndex?: unknown
  delta?: unknown
  daemonDisconnected?: unknown
  sessionId?: unknown
  statusLine?: unknown
  intervalMs?: unknown
  timeoutMs?: unknown
  selectedWorkflowAlias?: unknown
  shellEntryCount?: unknown
  workflowAlias?: unknown
}

type CliAutomationResponse = {
  id: string | number | null
  ok: boolean
  data?: unknown
  error?: string
}

function getLogger(component: string, fields: Record<string, unknown> = {}) {
  return processLogger?.child(component, fields) ?? null
}

async function inferWorkspaceTargetsFromLaunchDirectory(cwd: string): Promise<{ workspace: string; worktree: string }> {
  try {
    const [worktreeResult, commonDirResult] = await Promise.all([
      execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd }),
      execFileAsync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd }),
    ])
    const worktree = worktreeResult.stdout.trim()
    const commonDir = commonDirResult.stdout.trim()
    if (!worktree) {
      return { workspace: cwd, worktree: cwd }
    }
    const workspace = commonDir.endsWith("/.git")
      ? commonDir.slice(0, -"/.git".length)
      : worktree
    return { workspace, worktree }
  } catch {
    return { workspace: cwd, worktree: cwd }
  }
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

function isNoArgDefaultKernelLaunch(argv: string[]): boolean {
  return argv.length === 0
}

async function isKernelEndpointReachable(endpoint: string): Promise<boolean> {
  let url: URL
  try {
    url = new URL(endpoint)
  } catch {
    return true
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") {
    return true
  }
  const port = Number(url.port || (url.protocol === "wss:" ? 443 : 80))
  if (!Number.isFinite(port) || port <= 0) {
    return true
  }
  return new Promise((resolve) => {
    const socket = createConnection({ host: url.hostname, port })
    const settle = (reachable: boolean) => {
      socket.removeAllListeners()
      socket.destroy()
      resolve(reachable)
    }
    socket.setTimeout(500)
    socket.once("connect", () => settle(true))
    socket.once("error", () => settle(false))
    socket.once("timeout", () => settle(false))
  })
}

function isKernelEndpointUnavailableError(error: unknown): boolean {
  const message = formatError(error)
  return /\bECONNREFUSED\b|\bENOENT\b|\bEHOSTUNREACH\b|\bENETUNREACH\b|\bETIMEDOUT\b/i.test(message)
    || /kernel transport `connect kernel websocket` failed: \[object ErrorEvent\]/i.test(message)
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
  const [commandCenterQuery, setCommandCenterQuery] = createSignal("")
  const [commandCenterItems, setCommandCenterItems] = createSignal<CommandCenterItem[]>([])
  const [commandCenterIndex, setCommandCenterIndex] = createSignal(0)
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
  let stopRequestInFlight = false
  let promptInput: TextareaRenderable | undefined
  let hotkeysFocus: Renderable | null = null
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
  let responsePrimaryFooterParts: SplitPaneFooterTextGroup = {}
  const responseAuxiliaryFooterParts: Array<SplitPaneFooterTextGroup> = []
  let responsePrimaryFooterBadgeTexts: TextRenderable[] = []
  const responseAuxiliaryFooterBadgeTexts: Array<TextRenderable[]> = []
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
  let promptStateText: TextRenderable | undefined
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
  let footerSummaryText: TextRenderable | undefined
  let footerFlashText: TextRenderable | undefined
  let historyLoadingText: TextRenderable | undefined
  let statusOpenText: TextRenderable | undefined
  let statusCloseText: TextRenderable | undefined
  let statusLabelTexts: TextRenderable[] = []
  let closing = false
  let exitCleanupFailed = false
  const degradedPollers = new Set<string>()
  const tools = new Map<string, ToolTranscriptUpdate>()
  const activeToolLabels = new Map<string, string>()
  const transcriptRenderables = new Map<number, TranscriptEntryRenderable>()
  let transcriptSyntax = createTranscriptSyntaxStyle()
  let emptyTranscriptRenderable: BoxRenderable | undefined
  let footerFlashTimeout: ReturnType<typeof startTimeout> | undefined
  let lastTranscriptScrollTop = 0
  let historyLoadGeneration = 0
  let pendingHistoryScrollRestore = 0
  let pendingSessionChromeUpdate = false
  let pendingSessionChromeFlush: ReturnType<typeof startTimeout> | undefined
  let pendingTranscriptRender = false
  let pendingSplitPaneRefresh = 0
  let uiBatchDepth = 0
  let pendingTerminalRecordFlush: ReturnType<typeof startTimeout> | undefined
  let pendingTerminalRecords: TerminalOutputRecord[] = []
  let pendingTurnCompletion: ReturnType<typeof startTimeout> | undefined
  let turnCompletionConfirmed = false
  // Connection resilience tracking
  let lastDaemonActivityAt = Date.now()
  let lastTurnActivityAt = Date.now()
  let connectionWatchdogTimeout: ReturnType<typeof startTimeout> | undefined
  let consecutiveSilentPolls = 0
  const SILENT_POLL_THRESHOLD = 8 // ~2 seconds of no activity (8 * 250ms polling interval)
  let providerRecoveryInFlight = false
  let kernelResyncInFlight: Promise<void> | null = null
  let kernelRestartRecoveryInFlight: Promise<void> | null = null
  let subscribedSessionId: string | null = null
  let subscribedAttachmentId: string | null = null
  let subscribedScope: "session" | "waiting-room" | null = null
  let lastLoggedFocusedBadgeState: string | null = null
  let pendingAgentFocusTransition: Promise<void> | null = null
  let currentTurnId = computeCurrentTurnId(initialEntries)
  let nextTurnId = computeNextTurnId(initialEntries)
  let mountedTranscriptAgentId = initialBinding ? initialSession.focused_agent_id ?? initialSession.agents[0]?.id ?? null : null
  let hydratedPromptHistorySessionId: string | null | undefined
  let promptHistoryHydrationGeneration = 0
  let promptInputHistoryLatestSequence = 0
  let promptInputHistoryRefreshInFlight: Promise<void> | null = null
  let pendingPromptInputHistoryRefresh: ReturnType<typeof startTimeout> | undefined
  let promptTextSnapshot = initialPromptDraft
  let promptTextMuting = false
  let promptDropPending = false
  let pendingPromptDraftPersist: ReturnType<typeof startTimeout> | undefined
  let pendingPromptDraftSessionId: string | null = null
  let pendingPromptDraftValue = ""
  let submittingAgentId: string | null = null
  const renderScheduler = createRenderScheduler({
    schedule: (callback) => startTimeout(callback, 0),
    clearSchedule: clearTimeout,
    requestRootRender: () => {
      ;(renderer as { requestRender?: () => void }).requestRender?.()
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
  const agentBusyLatch = (agentId: string | null | undefined) => agentId ? (agentBusyLatches()[agentId] ?? false) : false
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
    if (!agentId) {
      return null
    }
    if (agentId === visibleTranscriptAgentId()) {
      return Array.from(activeToolLabels.values()).at(-1) ?? null
    }
    const toolState = agentPaneTools.get(agentId)
    if (!toolState) {
      return null
    }
    const labels = Array.from(toolState.values())
      .filter((update) => update.status !== "completed" && update.status !== "error" && update.status !== "cancelled")
      .map((update) => getToolActivityLabel(update.tool))
      .filter((label): label is string => Boolean(label))
    return labels.at(-1) ?? null
  }
  const focusedActivityLabel = () => {
    const agentId = focusedAgentId()
    const toolLabel = activeToolLabelForAgent(agentId)
    return toolLabel ?? agentActivityLabel(agentId)
  }
  const setAgentBusyLatch = (agentId: string | null | undefined, busy: boolean) => {
    if (!agentId) {
      return
    }
    setAgentBusyLatches((current) => {
      if ((current[agentId] ?? false) === busy) {
        return current
      }
      if (busy) {
        return {
          ...current,
          [agentId]: true,
        }
      }
      const next = { ...current }
      delete next[agentId]
      return next
    })
  }
  const markAgentBusy = (agentId: string | null | undefined) => {
    setAgentBusyLatch(agentId, true)
  }
  const clearAgentBusy = (agentId: string | null | undefined) => {
    setAgentBusyLatch(agentId, false)
  }
  const focusedAgentBusy = () => {
    const agentId = focusedAgentId()
    if (!agentId) {
      return false
    }
    const focused = sessionState().agents.find((agent) => agent.id === agentId) ?? null
    return (submitting() && submittingAgentId === agentId)
      || agentHasPromptWork(sessionState(), agentId)
      || streamingAgentId() === agentId
      || Boolean(focusedActivityLabel())
      || agentBusyLatch(agentId)
      || Boolean(focused && (focused.is_processing || focused.state === "Working"))
  }
  const allAgentsBusyState = () => {
    return sessionState().agents.map((agent) => {
      const agentId = agent.id
      const isBusy = (submitting() && submittingAgentId === agentId)
        || agentHasPromptWork(sessionState(), agentId)
        || streamingAgentId() === agentId
        || Boolean(agentActivityLabels()[agentId])
        || agentBusyLatch(agentId)
        || (agent.is_processing || agent.state === "Working")
      return { id: agentId, busy: isBusy }
    })
  }
  const shouldPreserveAgentActivityLabel = (agentId: string | null | undefined) => {
    if (!agentId) {
      return false
    }
    return streamingAgentId() === agentId
      || agentHasPromptWork(sessionState(), agentId)
      || sessionState().agents.some((agent) => agent.id === agentId && (agent.is_processing || agent.state === "Working"))
  }
  const setAgentActivityLabel = (agentId: string | null | undefined, nextLabel: string | null) => {
    if (!agentId) {
      return
    }
    setAgentActivityLabels((current) => ({
      ...current,
      [agentId]: nextLabel ?? (shouldPreserveAgentActivityLabel(agentId) ? (current[agentId] ?? null) : null),
    }))
  }
  const visibleTranscriptEntries = () => entries.filter((entry) => entry && !entry.hidden)
  const queueDepth = () => focusedQueueDepth()
  const connectedClientCount = () => sessionState().attachment_ids.length
  const activePrompt = () => focusedActivePrompt()
  const resolveTerminalRecordAgentId = (record: TerminalOutputRecord) => {
    if (record.agent_id) {
      return record.agent_id
    }
    const activeStreamAgentId = streamingAgentId()
    const activePromptAgentId = activePrompt()?.target_agent_id ?? null
    const activeProcessingAgentId = sessionState().agents.find((agent) => agent.is_processing || agent.state === "Working")?.id ?? null
    return record.agent_id
      ?? activeStreamAgentId
      ?? activePromptAgentId
      ?? activeProcessingAgentId
      ?? focusedAgentId()
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
    if (!renderable) {
      return null
    }
    const focusable = renderable as Renderable & { focused?: boolean }
    return {
      id: String((renderable as { id?: string | number }).id ?? ""),
      type: renderable.constructor?.name ?? null,
      destroyed: renderable.isDestroyed,
      focused: Boolean(focusable.focused),
    }
  }
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
      current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
    })
    toggleHotkeys()
    hotkeyDebug(`shortcut ${source} finished open=${hotkeysOpen()} saved=${describeRenderableDebug(hotkeysFocus)?.type ?? "none"}`)
    appLogger?.debug("finished toggling hotkeys via shortcut", {
      source,
      reason: hotkeysToggle.reason,
      previous_hotkeys_open: previousHotkeysOpen,
      hotkeys_open: hotkeysOpen(),
      saved_focus: describeRenderableDebug(hotkeysFocus),
      current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
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
      if (waitingRoomState().focus === "relay") {
        await handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] })
        return
      }
      if (waitingRoomState().focus === "workspace") {
        const command = `/workspace ${pendingWorkspaceTarget()}`
        setPromptText(command)
        promptInput?.focus()
        syncCommandCenter(command)
        flashFooter("edit the workspace path and press Enter", "info")
        return
      }
      if (waitingRoomState().focus === "worktree") {
        const command = `/worktree ${pendingWorktreeTarget()}`
        setPromptText(command)
        promptInput?.focus()
        syncCommandCenter(command)
        flashFooter("edit the worktree path and press Enter", "info")
        return
      }
      if (waitingRoomState().focus === "machine") {
        const machine = remoteMachinesState()[waitingRoomState().machineIndex]
        if (!machine) {
          flashFooter("no remote machine selected", "error")
          return
        }
        const label = machine.display_name ?? machine.registry_alias ?? machine.machine_alias ?? machine.machine_id
        if (machine.online === false || machine.pending || machine.kernel_count === 0) {
          flashFooter(`press D twice to delete machine ${label}`, "info")
          return
        }
        const command = `/machine kernels ${machine.registry_alias ?? machine.machine_alias ?? machine.machine_id}`
        setPromptText(command)
        promptInput?.focus()
        syncCommandCenter(command)
        flashFooter(`press Enter to list kernels for ${label}`, "info")
        return
      }
      if (waitingRoomState().focus === "remote-kernel") {
        const kernel = remoteKernelsState()[waitingRoomState().remoteKernelIndex]
        if (!kernel) {
          flashFooter("no remote kernel selected", "error")
          return
        }
        const target = kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
        if (!waitingRoomRemoteKernelIsAttachable(kernel)) {
          flashFooter(
            waitingRoomRemoteKernelCanDelete(kernel)
              ? `press D twice to delete kernel ${target}`
              : `kernel ${target} is active`,
            waitingRoomRemoteKernelCanDelete(kernel) ? "info" : "error",
          )
          return
        }
        const command = `/relay cloud client-token ${target}`
        setPromptText(command)
        promptInput?.focus()
        syncCommandCenter(command)
        flashFooter(`press Enter to mint a relay token for ${target}`, "info")
        return
      }
      if (waitingRoomState().focus === "terminal") {
        const terminal = terminalsState()[waitingRoomState().terminalIndex]
        if (!terminal) {
          flashFooter("no terminal selected", "error")
          return
        }
        flashFooter(`${terminal.terminal_id} is a ${formatTerminalTypeLabel(terminal.terminal_type)}`, "info")
        return
      }
      if (waitingRoomState().focus === "add-terminal") {
        await openTerminalPairingDialog()
        return
      }
      if (waitingRoomState().focus === "join-sessions") {
        openSessionBrowserDialog()
        return
      }
      const decision = deriveWaitingRoomActivationDecision({
        state: waitingRoomState(),
        sessions: availableSessions(),
        catalog: providerCatalogState(),
        currentProvider: (options.provider ?? "opencode") as BackendProviderId,
        currentModel: options.model,
        remote: {
          relay: relayStatusState(),
          machines: remoteMachinesState(),
          kernels: remoteKernelsState(),
          terminals: terminalsState(),
          slices: slicesState(),
        },
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
  let waitingRoomDataRefresh: Promise<void> | null = null
  let waitingRoomInventoryVersion: string | null = null
  let pendingWaitingRoomSessionAction: PendingWaitingRoomSessionAction | null = null
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
        pendingWaitingRoomSessionAction = null
        flashFooter(decision.message, "error")
        return
      }

      const target = waitingRoomLifecycleTarget(decision)
      const now = Date.now()
      const pending = pendingWaitingRoomSessionAction
      const keyLabel = action === "archive" ? "A" : "D"
      if (
        !pending
        || pending.action !== action
        || pending.targetKind !== target.kind
        || pending.targetId !== target.id
        || pending.expiresAtMs <= now
      ) {
        pendingWaitingRoomSessionAction = {
          action,
          targetKind: target.kind,
          targetId: target.id,
          expiresAtMs: now + WAITING_ROOM_SESSION_ACTION_CONFIRM_MS,
        }
        flashFooter(`press ${keyLabel} again to ${target.verb} ${target.label}`, action === "delete" ? "error" : "info")
        return
      }

      pendingWaitingRoomSessionAction = null
      if (decision.action === "archive") {
        const updated = await archiveSessionById(client, decision.session.id)
        setAvailableSessions(availableSessions().filter((candidate) => candidate.id !== updated.id))
        waitingRoomInventoryVersion = null
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
        waitingRoomInventoryVersion = null
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
        waitingRoomInventoryVersion = null
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
        waitingRoomInventoryVersion = null
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
        waitingRoomInventoryVersion = null
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
        waitingRoomInventoryVersion = null
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
  const waitingRoomSessionLifecycleActionForEvent = (event: {
    name: string
    eventType?: string
    ctrl?: boolean
    meta?: boolean
    alt?: boolean
    super?: boolean
  }): WaitingRoomSessionLifecycleAction | null => {
    if (event.eventType === "release" || event.ctrl || event.meta || event.alt || event.super || promptInput?.focused) {
      return null
    }
    if (event.name === "a") {
      return "archive"
    }
    if (event.name === "d" || event.name === "delete") {
      return "delete"
    }
    return null
  }
  const handleSessionBrowserKey = (event: {
    name: string
    eventType?: string
    ctrl?: boolean
    meta?: boolean
    alt?: boolean
    super?: boolean
  }) => {
    if (!sessionBrowserOpen() || event.eventType === "release" || event.ctrl || event.meta || event.alt || event.super) {
      return false
    }
    const sessions = sessionBrowserSessions()
    if (event.name === "escape") {
      closeSessionBrowserDialog()
      return true
    }
    if (event.name === "up" || event.name === "down") {
      if (sessions.length > 0) {
        setSessionBrowserIndex((index) => {
          const next = event.name === "up" ? index - 1 : index + 1
          return ((next % sessions.length) + sessions.length) % sessions.length
        })
        renderHotkeysOverlay()
      }
      return true
    }
    const selectedIndex = normalizeSessionBrowserIndex()
    const selected = sessions[selectedIndex]
    if (!selected) {
      flashFooter("no sessions available", "error")
      return true
    }
    if (event.name === "return" || event.name === "enter") {
      const state = { ...waitingRoomState(), focus: "session" as const, sessionIndex: selectedIndex }
      const decision = deriveWaitingRoomActivationDecision({
        state,
        sessions: availableSessions(),
        catalog: providerCatalogState(),
        currentProvider: (options.provider ?? "opencode") as BackendProviderId,
        currentModel: options.model,
      })
      if (decision.action !== "join") {
        flashFooter(decision.action === "error" ? decision.message : "select a session to join", "error")
        return true
      }
      closeSessionBrowserDialog()
      void attachBinding(decision.session, false, decision.launch).then(
        () => flashFooter(`attached to session ${decision.session.alias ?? decision.session.id}`, "info"),
        (error) => flashFooter(formatError(error), "error"),
      )
      return true
    }
    const lifecycleAction = event.name === "a"
      ? "archive"
      : event.name === "d" || event.name === "delete"
        ? "delete"
        : null
    if (lifecycleAction) {
      void applyWaitingRoomSessionLifecycleAction(lifecycleAction, {
        ...waitingRoomState(),
        focus: "session",
        sessionIndex: selectedIndex,
      }).then(() => {
        const nextLength = sessionBrowserSessions().length
        if (nextLength === 0) {
          closeSessionBrowserDialog()
        } else {
          setSessionBrowserIndex((index) => Math.min(index, nextLength - 1))
          renderHotkeysOverlay()
        }
      })
      return true
    }
    return true
  }
  const waitingRoomLifecycleTarget = (
    decision: WaitingRoomSessionLifecycleDecision | WaitingRoomDeleteDecision,
  ) => {
    if (decision.action === "archive") {
      return {
        kind: "session" as const,
        id: decision.session.id,
        label: `session ${formatSessionDisplayLabel(decision.session)}`,
        verb: "archive",
      }
    }
    if (decision.action === "archive-all") {
      return {
        kind: "sessions" as const,
        id: "all",
        label: `${decision.sessions.length} session${decision.sessions.length === 1 ? "" : "s"}`,
        verb: "archive",
      }
    }
    if (decision.action === "delete-session") {
      return {
        kind: "session" as const,
        id: decision.session.id,
        label: `session ${formatSessionDisplayLabel(decision.session)}`,
        verb: "delete",
      }
    }
    if (decision.action === "delete-all-sessions") {
      return {
        kind: "sessions" as const,
        id: "all",
        label: `${decision.sessions.length} session${decision.sessions.length === 1 ? "" : "s"}`,
        verb: "delete",
      }
    }
    if (decision.action === "delete") {
      return {
        kind: "session" as const,
        id: decision.session.id,
        label: `session ${formatSessionDisplayLabel(decision.session)}`,
        verb: "delete",
      }
    }
    if (decision.action === "delete-machine") {
      return {
        kind: "machine" as const,
        id: decision.machineId,
        label: `machine ${decision.label}`,
        verb: "delete",
      }
    }
    if (decision.action === "delete-kernel") {
      return {
        kind: "kernel" as const,
        id: decision.kernelId,
        label: `kernel ${decision.label}`,
        verb: "delete",
      }
    }
    throw new Error("unsupported waiting room lifecycle decision")
  }
  const connectDetachedKernelFromWaitingRoom = async () => {
    appLogger?.info("connecting detached cli to configured kernel endpoint")
    flashFooter("connecting to kernel...", "info")
    const [catalog, commandCatalogs] = await Promise.all([
      getProviderCatalog(client, appLogger),
      getProviderCommandCatalogs(client, appLogger),
    ])
    waitingRoomInventoryVersion = null
    setProviderCatalogState(catalog)
    setProviderCommandCatalogState(commandCatalogs)
    setKernelConnected(true)
    setDaemonDisconnected(false)
    await refreshWaitingRoomData()
    flashFooter("connected to kernel", "info")
  }
  const refreshWaitingRoomDataNow = async () => {
    if (!kernelConnected()) {
      return
    }
    if (waitingRoomInventoryStatus() !== "ready") {
      setWaitingRoomInventoryStatus("loading")
    }
    const snapshot = await getWaitingRoomInventory(client).catch((error) => {
      appLogger?.warn("waiting room inventory refresh failed", { error: formatError(error) })
      setWaitingRoomInventoryStatus("error")
      return null
    })
    if (!snapshot) {
      return
    }
    setWaitingRoomInventoryStatus("ready")
    if (snapshot.inventoryVersion === waitingRoomInventoryVersion) {
      reconcileWaitingRoom(waitingRoomState())
      return
    }
    waitingRoomInventoryVersion = snapshot.inventoryVersion
    setAvailableSessions(snapshot.sessions)
    setRelayStatusState(snapshot.relayStatus)
    setRemoteMachinesState(snapshot.remoteMachines)
    setRemoteKernelsState(snapshot.remoteKernels.filter((kernel) => (
      !hiddenWaitingRoomKernelIds.has(kernel.kernel_id)
      || !waitingRoomRemoteKernelCanDelete(kernel)
    )))
    setTerminalsState(snapshot.terminals)
    setSlicesState(snapshot.slices)
    reconcileWaitingRoom(waitingRoomState())
  }
  const refreshWaitingRoomData = async () => {
    if (waitingRoomDataRefresh) {
      return waitingRoomDataRefresh
    }
    waitingRoomDataRefresh = refreshWaitingRoomDataNow().finally(() => {
      waitingRoomDataRefresh = null
    })
    return waitingRoomDataRefresh
  }
  const applyModelSelection = async (modelId: string) => {
    const currentSelection = currentProviderSelection()
    const decision = deriveWaitingRoomModelSelectionDecision({
      modelId,
      state: waitingRoomState(),
      sessions: availableSessions(),
      catalog: providerCatalogState(),
      themeRegistry: themeRegistryState(),
      currentProvider: normalizeBackendProviderId(currentSelection.provider),
      configuredEffort: currentSelection.effort,
    })
    if (decision.kind === "error") {
      flashFooter(decision.message, "error")
      return
    }
    reconcileWaitingRoom(decision.nextState)
    if (!isAttached()) {
      flashFooter(`selected model ${decision.selectedModelId}`, "info")
      return
    }
    const agentId = focusedAgentId()
    if (!agentId) {
      flashFooter("no focused agent to update", "error")
      return
    }
    const activeRun = providerRunState()
    if (activeRun?.agent_instance_id === agentId && providerRunUsesNativeTui(activeRun)) {
      flashFooter("model is controlled by the provider-native TUI for this agent", "error")
      return
    }
    const payload = await updateAgentProfile(
      client,
      sessionState().id,
      agentId,
      {
        provider: decision.launch.provider,
        model: decision.launch.model,
        effort: decision.launch.effort,
      },
    )
    applySessionState(payload.session)
    setProviderRunState(null)
    flashFooter(`model set to ${decision.selectedModelId}`, "info")
  }
  const applyVariantSelection = async (variant: string) => {
    const currentSelection = currentProviderSelection()
    const decision = deriveWaitingRoomVariantSelectionDecision({
      variant,
      currentModelId: currentSelection.model,
      currentProviderId: normalizeBackendProviderId(currentSelection.provider),
      state: waitingRoomState(),
      sessions: availableSessions(),
      catalog: providerCatalogState(),
      themeRegistry: themeRegistryState(),
    })
    if (decision.kind === "error") {
      flashFooter(decision.message, "error")
      return
    }
    reconcileWaitingRoom(decision.nextState)
    if (!isAttached()) {
      flashFooter(`selected variant ${decision.selectedVariant}`, "info")
      return
    }
    const agentId = focusedAgentId()
    if (!agentId) {
      flashFooter("no focused agent to update", "error")
      return
    }
    const activeRun = providerRunState()
    if (activeRun?.agent_instance_id === agentId && providerRunUsesNativeTui(activeRun)) {
      flashFooter("variant is controlled by the provider-native TUI for this agent", "error")
      return
    }
    const payload = await updateAgentProfile(
      client,
      sessionState().id,
      agentId,
      {
        provider: decision.launch.provider,
        model: decision.launch.model,
        effort: decision.launch.effort,
      },
    )
    applySessionState(payload.session)
    setProviderRunState(null)
    flashFooter(`variant set to ${decision.selectedVariant}`, "info")
  }
  const applyProviderSelection = async (providerId: string) => {
    if (!isBackendProviderId(providerId)) {
      flashFooter(`unknown provider: ${providerId}`, "error")
      return
    }
    options.provider = providerId
    const saved = preferencesState().providers?.[providerId]
    const selected = selectConfiguredModel(
      providerCatalogState(),
      saved?.model ?? options.model,
      providerId,
    )
    if (selected) {
      options.model = selected.id
    }
    if (saved?.effort != null) {
      options.effort = saved.effort
    } else if (selected) {
      options.effort = selectConfiguredVariant(selected, options.effort)
    }
    reconcileWaitingRoom({
      ...waitingRoomState(),
      providerId,
      modelId: options.model,
      effort: options.effort,
    })
    if (isAttached() && attachmentState()) {
      try {
        const agentId = focusedAgentId()
        if (!agentId) {
          flashFooter("no focused agent to update", "error")
          return
        }
        const payload = await updateAgentProfile(
          client,
          sessionState().id,
          agentId,
          {
            provider: providerId,
            model: options.model,
            effort: options.effort,
          },
        )
        applySessionState(payload.session)
        setProviderRunState(null)
      } catch (error) {
        appLogger?.warn("provider profile update failed", {
          provider: providerId,
          error: formatError(error),
        })
        flashFooter(formatError(error), "error")
        return
      }
    }
    if (providerId === "codex") {
      try {
        const status = await getProviderAuthStatus(client, providerId)
        if (status.auth_state !== "authenticated") {
          appendNotice(
            [
              "Codex is not logged in.",
              status.login_hint ?? "Run /provider login codex to authenticate.",
            ].join(" "),
          )
        }
      } catch (error) {
        appLogger?.warn("provider auth status lookup failed after selection", {
          provider: providerId,
          error: formatError(error),
        })
      }
    }
    flashFooter(`${backendProviderLabel(providerId)} selected`, "info")
  }
  const currentProviderSelection = () => deriveCurrentProviderSelection({
    providerRun: focusedProviderRun(),
    focusedAgent: focusedAgent(),
    waitingRoomState: waitingRoomState(),
    defaultProvider: options.provider ?? "opencode",
    defaultModel: options.model,
    defaultEffort: options.effort,
  })
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
  const syncCommandCenter = (value = promptInput?.plainText ?? promptTextSnapshot) => {
    const previousValue = commandCenterQuery()
    setCommandCenterQuery(value)
    const items = buildCommandCenterItems(value, {
      providerCatalog: providerCatalogState(),
      providerCommandCatalogs: providerCommandCatalogState(),
      currentProvider: normalizeBackendProviderId(currentProviderSelection().provider),
      focusedProvider: focusedBackendProvider(),
      currentModel: currentModelId(),
      currentVariant: currentVariantId(),
    })
    setCommandCenterItems(items)
    setCommandCenterIndex((index) => nextCommandCenterIndex(index, items, value, previousValue))
    renderCommandCenter()
  }
  const commandCenterOpen = () => commandCenterItems().length > 0 && commandCenterQuery().startsWith("/")
  const positionCommandCenter = () => {
    if (!commandCenterBox) {
      return
    }
    commandCenterBox.position = "absolute"
    commandCenterBox.left = 0
    commandCenterBox.right = 0
    commandCenterBox.bottom = (promptInput?.height ?? 1) + COMMAND_CENTER_OVERLAY_FOOTPRINT
    commandCenterBox.zIndex = 10
  }
  const selectCommandCenterItem = async (item: CommandCenterItem) => {
    if (item.kind === "command") {
      try {
        clearCommandCenter()
        setPromptText("")
        syncPromptTextSnapshot()
        await executeCommandCenterCommand(item.value)
      } catch (error) {
        flashFooter(formatError(error), "error")
      }
      return
    }
    if (item.kind === "group") {
      if (!item.value.endsWith(" ")) {
        try {
          clearCommandCenter()
          setPromptText("")
          syncPromptTextSnapshot()
          await executeCommandCenterCommand(item.value)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
        return
      }
      setPromptText(item.value)
      if (promptInput) {
        promptInput.cursorOffset = item.value.length
      }
      syncPromptTextSnapshot()
      syncCommandCenter(item.value)
      return
    }
    if (item.kind === "provider") {
      try {
        await executeCommandCenterCommand(`/provider ${item.value}`)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        setPromptText("")
        syncPromptTextSnapshot()
        syncCommandCenter("")
      }
      return
    }
    if (item.kind === "model") {
      try {
        await executeCommandCenterCommand(`/model ${item.value}`)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        setPromptText("")
        syncPromptTextSnapshot()
        syncCommandCenter("")
      }
      return
    }
    if (item.kind === "variant") {
      try {
        await executeCommandCenterCommand(`/variant ${item.value}`)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        setPromptText("")
        syncPromptTextSnapshot()
        syncCommandCenter("")
      }
    }
  }
  const completeCommandCenterItem = (item: CommandCenterItem) => {
    if (item.kind === "command" || item.kind === "group") {
      setPromptText(item.value)
      if (promptInput) {
        promptInput.cursorOffset = item.value.length
      }
      syncPromptTextSnapshot()
      syncCommandCenter(item.value)
      return
    }
    if (item.kind === "provider") {
      const command = `/provider ${item.value}`
      setPromptText(command)
      if (promptInput) {
        promptInput.cursorOffset = command.length
      }
      syncPromptTextSnapshot()
      syncCommandCenter(command)
      return
    }
    if (item.kind === "model") {
      const command = `/model ${item.value}`
      setPromptText(command)
      if (promptInput) {
        promptInput.cursorOffset = command.length
      }
      syncPromptTextSnapshot()
      syncCommandCenter(command)
      return
    }
    if (item.kind === "variant") {
      const command = `/variant ${item.value}`
      setPromptText(command)
      if (promptInput) {
        promptInput.cursorOffset = command.length
      }
      syncPromptTextSnapshot()
      syncCommandCenter(command)
    }
  }
  const moveCommandCenterSelection = (delta: number) => {
    const items = commandCenterItems()
    if (items.length === 0) {
      return
    }
    setCommandCenterIndex((index) => (index + delta + items.length) % items.length)
  }
  const clearCommandCenter = () => {
    setCommandCenterQuery("")
    setCommandCenterItems([])
    setCommandCenterIndex(0)
    renderCommandCenter()
  }
  const selectedCommandCenterItem = () => commandCenterItems()[commandCenterIndex()] ?? commandCenterItems()[0] ?? null
  const commandCenterVisibleRowCount = () => Math.max(4, Math.min(10, dimensions().height - (promptInput?.height ?? 1) - 10))
  const handleCommandCenterKey = (event: {
    name: string
    ctrl?: boolean
    eventType?: string
    preventDefault?: () => void
    stopPropagation?: () => void
  }) => {
    if (!commandCenterOpen() || event.eventType === "release") {
      return false
    }
    if (event.name === "up" || (event.ctrl && event.name === "p")) {
      event.preventDefault?.()
      event.stopPropagation?.()
      moveCommandCenterSelection(-1)
      renderCommandCenter()
      return true
    }
    if (event.name === "down" || (event.ctrl && event.name === "n")) {
      event.preventDefault?.()
      event.stopPropagation?.()
      moveCommandCenterSelection(1)
      renderCommandCenter()
      return true
    }
    if (event.name === "escape") {
      event.preventDefault?.()
      event.stopPropagation?.()
      clearCommandCenter()
      return true
    }
    if (event.name === "return" || event.name === "enter") {
      const item = selectedCommandCenterItem()
      if (!item) {
        return false
      }
      event.preventDefault?.()
      event.stopPropagation?.()
      void selectCommandCenterItem(item)
      return true
    }
    if (event.name === "tab") {
      const item = selectedCommandCenterItem()
      if (!item) {
        return false
      }
      event.preventDefault?.()
      event.stopPropagation?.()
      completeCommandCenterItem(item)
      return true
    }
    return false
  }
  const selectCommandCenterFromSubmit = () => {
    const isSessionAliasPrompt = (prompt: string) => {
      const command = parseSlashCommand(prompt)
      if (!command || command.kind !== "session" || command.action === null) {
        return false
      }
      const action = command.action.toLowerCase()
      if (action === "new" || action === "create" || action === "attach" || action === "list" || action === "ls" || action === "delete") {
        return false
      }
      return true
    }
    const item = selectedCommandCenterItem()
    if (!item) {
      return false
    }
    const currentPrompt = promptInput?.plainText ?? ""
    if (isSessionAliasPrompt(currentPrompt)) {
      return false
    }
    // Leaf commands like `/session attach ` should submit once fully typed.
    // Parent groups like `/workflow` should expand to their subcommands instead.
    if (shouldSubmitExactCommandCenterMatch(item, currentPrompt)) {
      clearCommandCenter()
      syncCommandCenter("")
      return false
    }
    void selectCommandCenterItem(item)
    return true
  }
  const renderCommandCenter = () => {
    if (!commandCenterBox) {
      return
    }
    positionCommandCenter()
    for (const child of [...commandCenterBox.getChildren()]) {
      commandCenterBox.remove(child.id)
      child.destroyRecursively()
    }
    if (!commandCenterOpen()) {
      commandCenterBox.requestRender()
      return
    }

    const panel = new BoxRenderable(renderer, {
      flexDirection: "column",
      border: ["left"],
      borderColor: theme.primary,
      customBorderChars: SplitBorder.customBorderChars,
      paddingLeft: 1,
      paddingTop: 1,
      paddingBottom: 1,
      backgroundColor: theme.backgroundPanel,
      gap: 0,
    })

    const items = commandCenterItems()
    const selectedIndex = Math.min(commandCenterIndex(), Math.max(0, items.length - 1))
    const visibleRowCount = commandCenterVisibleRowCount()
    const windowStart = Math.max(0, Math.min(selectedIndex - Math.floor(visibleRowCount / 2), Math.max(0, items.length - visibleRowCount)))
    const visibleItems = items.slice(windowStart, windowStart + visibleRowCount)

    if (windowStart > 0) {
      panel.add(new TextRenderable(renderer, {
        content: `  ${windowStart} more above`,
        fg: theme.textMuted,
        wrapMode: "none",
      }))
    }

    for (const [offset, item] of visibleItems.entries()) {
      const index = windowStart + offset
      const selected = index === selectedIndex
      const row = new BoxRenderable(renderer, {
        flexDirection: "row",
        justifyContent: "space-between",
        paddingLeft: 1,
        paddingRight: 1,
        ...(selected ? { backgroundColor: theme.primary } : {}),
      })
      row.add(new TextRenderable(renderer, {
        content: item.kind === "group" ? `${item.label} >` : item.label,
        fg: selected ? theme.background : theme.text,
        attributes: selected ? TextAttributes.BOLD : TextAttributes.NONE,
        wrapMode: "none",
      }))
      row.add(new TextRenderable(renderer, {
        content: item.description,
        fg: selected ? theme.background : theme.textMuted,
        wrapMode: "none",
      }))
      panel.add(row)
    }

    const hiddenBelow = items.length - (windowStart + visibleItems.length)
    if (hiddenBelow > 0) {
      panel.add(new TextRenderable(renderer, {
        content: `  ${hiddenBelow} more below`,
        fg: theme.textMuted,
        wrapMode: "none",
      }))
    }

    commandCenterBox.add(panel)
    commandCenterBox.requestRender()
  }
  const cancelPendingTurnCompletion = () => {
    if (pendingTurnCompletion) {
      clearTimeout(pendingTurnCompletion)
      pendingTurnCompletion = undefined
    }
  }
  const recordTurnActivity = (_activityType: string) => {
    lastTurnActivityAt = Date.now()
    cancelPendingTurnCompletion()
  }
  const turnCompletionDelayMs = () => getTurnCompletionDelayMs({
    sessionHasPromptWork: sessionHasPromptWork(sessionState()),
    pendingTerminalRecordCount: pendingTerminalRecords.length,
    pendingTerminalRecordFlush: Boolean(pendingTerminalRecordFlush),
    lastTurnActivityAt,
    now: Date.now(),
    quietWindowMs: TURN_COMPLETION_QUIET_MS,
  })
  const maybeScheduleConfirmedTurnCompletion = () => {
    if (!turnCompletionConfirmed || activePrompt()) {
      return
    }
    scheduleTurnCompletion()
  }
  const finalizeTurnCompletion = () => {
    cancelPendingTurnCompletion()
    const delayMs = turnCompletionDelayMs()
    if (delayMs === null) {
      return
    }
    if (delayMs > 0) {
      scheduleTurnCompletion()
      return
    }
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
    turnCompletionConfirmed = false
    renderSessionChromeBoundary()
  }
  const scheduleTurnCompletion = () => {
    cancelPendingTurnCompletion()
    const delayMs = turnCompletionDelayMs()
    if (delayMs === null) {
      return
    }
    pendingTurnCompletion = startTimeout(() => {
      pendingTurnCompletion = undefined
      finalizeTurnCompletion()
    }, delayMs)
  }
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
    promptTextSnapshot = nextDraft
    setPromptText(nextDraft)
  }
  const hydratePromptHistoryFromSession = async (sessionId: string) => {
    const generation = ++promptHistoryHydrationGeneration
    await loadAndApplyPromptHistoryFromSession(sessionId, generation)
  }
  const loadAndApplyPromptHistoryFromSession = async (
    sessionId: string,
    generation: number,
  ) => {
    const promptInputHistory = await getPromptInputHistory(client, sessionId)

    if (generation !== promptHistoryHydrationGeneration) {
      return
    }
    if (attachmentState()?.session_id !== sessionId) {
      return
    }

    const nextEntries = extractPromptInputHistoryEntries(promptInputHistory.entries)
    promptInputHistoryLatestSequence = maxPromptInputHistorySequence(promptInputHistory.entries)

    setPromptHistoryEntries(nextEntries)
    setPromptHistoryIndex(null)
    setPromptHistoryDraft(null)
    setPreferencesState((current) => mergeSessionPromptState(current, sessionId, {
      promptHistory: nextEntries,
    }))
    await saveSessionPromptState(sessionId, { promptHistory: nextEntries })
  }
  const appendSharedPromptInputHistory = (
    sessionId: string,
    entries: readonly PromptInputHistoryPage["entries"][number][],
  ) => {
    if (attachmentState()?.session_id !== sessionId || entries.length === 0) {
      return
    }
    const currentEntries = promptHistoryEntries()
    let nextEntries = currentEntries
    for (const entry of [...entries].sort((left, right) => left.sequence - right.sequence)) {
      promptInputHistoryLatestSequence = Math.max(promptInputHistoryLatestSequence, entry.sequence)
      nextEntries = pushPromptHistoryEntry(nextEntries, entry.text)
    }
    if (promptHistoryEntryListsEqual(nextEntries, currentEntries)) {
      return
    }
    setPromptHistoryEntries(nextEntries)
    void persistSessionPromptState(sessionId, {
      promptHistory: nextEntries,
    }).catch((error) => {
      appLogger?.warn("failed to persist shared prompt input history", {
        session_id: sessionId,
        error: formatError(error),
      })
    })
  }
  const appendPromptEchoToSharedHistory = (text: string) => {
    const sessionId = attachmentState()?.session_id
    if (!sessionId) {
      return
    }
    const currentEntries = promptHistoryEntries()
    const nextPromptHistoryEntries = pushPromptHistoryEntry(currentEntries, text)
    if (promptHistoryEntryListsEqual(nextPromptHistoryEntries, currentEntries)) {
      return
    }
    setPromptHistoryEntries(nextPromptHistoryEntries)
    void persistSessionPromptState(sessionId, {
      promptHistory: nextPromptHistoryEntries,
    }).catch((error) => {
      appLogger?.warn("failed to persist prompt echo history", {
        session_id: sessionId,
        error: formatError(error),
      })
    })
  }
  const refreshSharedPromptInputHistory = async (sessionId: string) => {
    if (promptInputHistoryRefreshInFlight) {
      return promptInputHistoryRefreshInFlight
    }
    promptInputHistoryRefreshInFlight = getPromptInputHistory(client, sessionId, promptInputHistoryLatestSequence, 500)
      .then((history) => {
        appendSharedPromptInputHistory(sessionId, history.entries)
      })
      .finally(() => {
        promptInputHistoryRefreshInFlight = null
      })
    return promptInputHistoryRefreshInFlight
  }
  const scheduleSharedPromptInputHistoryRefresh = () => {
    const sessionId = attachmentState()?.session_id
    if (!sessionId || pendingPromptInputHistoryRefresh) {
      return
    }
    pendingPromptInputHistoryRefresh = startTimeout(() => {
      pendingPromptInputHistoryRefresh = undefined
      void refreshSharedPromptInputHistory(sessionId).catch((error) => {
        appLogger?.warn("failed to refresh shared prompt input history", {
          session_id: sessionId,
          error: formatError(error),
        })
      })
    }, 1500)
  }
  const clearPendingPromptDraftPersist = () => {
    if (pendingPromptDraftPersist) {
      clearTimeout(pendingPromptDraftPersist)
      pendingPromptDraftPersist = undefined
    }
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
  const flushPendingPromptDraftPersist = async () => {
    clearPendingPromptDraftPersist()
    if (!pendingPromptDraftSessionId) {
      return
    }
    const sessionId = pendingPromptDraftSessionId
    const promptDraft = pendingPromptDraftValue
    pendingPromptDraftSessionId = null
    pendingPromptDraftValue = ""
    await persistSessionPromptState(sessionId, { promptDraft })
  }
  const persistablePromptDraft = () => {
    if (promptHistoryDraft() !== null) {
      return promptHistoryDraft() ?? ""
    }
    return promptInput?.plainText ?? promptTextSnapshot
  }
  const schedulePromptDraftPersist = (sessionId: string, promptDraft: string) => {
    pendingPromptDraftSessionId = sessionId
    pendingPromptDraftValue = promptDraft
    clearPendingPromptDraftPersist()
    pendingPromptDraftPersist = startTimeout(() => {
      pendingPromptDraftPersist = undefined
      const queuedSessionId = pendingPromptDraftSessionId
      if (!queuedSessionId) {
        return
      }
      const queuedDraft = pendingPromptDraftValue
      pendingPromptDraftSessionId = null
      pendingPromptDraftValue = ""
      void persistSessionPromptState(queuedSessionId, { promptDraft: queuedDraft }).catch((error) => {
        appLogger?.warn("failed to persist prompt draft", {
          session_id: queuedSessionId,
          error: formatError(error),
        })
      })
    }, 300)
  }
  const recordPromptAreaHistoryEntry = (sessionId: string | null, rawPrompt: string) => {
    if (!sessionId) {
      return
    }
    const nextPromptHistoryEntries = pushPromptHistoryEntry(promptHistoryEntries(), rawPrompt)
    setPromptHistoryEntries(nextPromptHistoryEntries)
    setPromptHistoryIndex(null)
    setPromptHistoryDraft(null)
    pendingPromptDraftSessionId = null
    pendingPromptDraftValue = ""
    clearPendingPromptDraftPersist()
    void persistSessionPromptState(sessionId, {
      promptHistory: nextPromptHistoryEntries,
      promptDraft: "",
    }).catch((error) => {
      appLogger?.warn("failed to persist session prompt state", {
        session_id: sessionId,
        error: formatError(error),
      })
    })
    const attachmentId = attachmentState()?.id ?? null
    if (rawPrompt.trimStart().startsWith("/")) {
      void client.send<Record<string, unknown>>(recordPromptInputHistoryRequest(
        sessionId,
        attachmentId,
        "command",
        rawPrompt.trimEnd(),
      )).then((response) => {
        const payload = expectVariant<{ entry: PromptInputHistoryPage["entries"][number] }>(
          response,
          "PromptInputHistoryRecorded",
        )
        appendSharedPromptInputHistory(sessionId, [payload.entry])
      }).catch((error) => {
        appLogger?.warn("failed to record shared prompt input history", {
          session_id: sessionId,
          error: formatError(error),
        })
      })
    }
  }
  const syncPromptTextSnapshot = () => {
    promptTextSnapshot = promptInput?.plainText ?? ""
  }
  const refreshPromptAttachmentHighlights = () => {
    if (!promptInput) {
      return
    }
    promptInput.clearAllHighlights()
    const value = promptInput.plainText
    for (const file of pendingAttachments()) {
      let start = value.indexOf(file.token)
      while (start !== -1) {
        promptInput.addHighlightByCharRange({
          start,
          end: start + file.token.length,
          styleId: promptTokenStyleIds[attachmentTokenKind(file.kind)],
        })
        start = value.indexOf(file.token, start + file.token.length)
      }
    }
  }
  const setPromptText = (value: string) => {
    if (!promptInput) {
      promptTextSnapshot = value
      return
    }
    promptTextMuting = true
    promptInput.setText(value)
    promptInput.cursorOffset = value.length
    promptTextSnapshot = value
    refreshPromptAttachmentHighlights()
    promptTextMuting = false
  }
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
  const navigatePromptHistoryInput = (direction: "previous" | "next") => {
    const currentText = promptInput?.plainText ?? promptTextSnapshot
    const entries = promptHistoryEntries()
    const next = navigatePromptHistory({
      entries,
      currentText,
      navigationIndex: promptHistoryIndex(),
      navigationDraft: promptHistoryDraft(),
      direction,
    })
    if (next.navigationIndex === promptHistoryIndex() && next.text === currentText) {
      return false
    }
    setPromptHistoryIndex(next.navigationIndex)
    setPromptHistoryDraft(next.navigationDraft)
    setPromptText(next.text)
    const sessionId = attachmentState()?.session_id
    if (sessionId) {
      schedulePromptDraftPersist(sessionId, next.navigationDraft ?? next.text)
    }
    retainPromptFocus()
    return true
  }
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
  const clearPendingPromptAttachments = () => {
    setPendingAttachments([])
    refreshPromptAttachmentHighlights()
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }

  type SubmittedPromptUiSnapshot = {
    rawPrompt: string
    attachments: PendingPromptAttachment[]
    sessionId: string | null
  }

  const beginSubmittedPromptUi = (rawPrompt: string): SubmittedPromptUiSnapshot => {
    const snapshot: SubmittedPromptUiSnapshot = {
      rawPrompt,
      attachments: pendingAttachments().map((file) => ({ ...file })),
      sessionId: attachmentState()?.session_id ?? null,
    }
    setPromptHistoryIndex(null)
    setPromptHistoryDraft(null)
    pendingPromptDraftSessionId = null
    pendingPromptDraftValue = ""
    clearPendingPromptDraftPersist()
    if (promptInput) {
      promptTextMuting = true
      promptInput.clear()
      promptInput.cursorOffset = 0
      promptTextSnapshot = ""
      promptTextMuting = false
    } else {
      setPromptText("")
    }
    syncPromptTextSnapshot()
    clearPendingPromptAttachments()
    syncCommandCenter("")
    retainPromptFocus()
    clearCommandCenter()
    if (snapshot.sessionId) {
      schedulePromptDraftPersist(snapshot.sessionId, "")
    }
    return snapshot
  }

  const restoreFailedPromptUi = (snapshot: SubmittedPromptUiSnapshot | null | undefined) => {
    if (!snapshot) {
      return
    }
    setPromptHistoryIndex(null)
    setPromptHistoryDraft(null)
    setPendingAttachments(snapshot.attachments.map((file) => ({ ...file })))
    setPromptText(snapshot.rawPrompt)
    syncPromptTextSnapshot()
    refreshPromptAttachmentHighlights()
    syncCommandCenter(snapshot.rawPrompt)
    retainPromptFocus()
    if (snapshot.sessionId) {
      schedulePromptDraftPersist(snapshot.sessionId, snapshot.rawPrompt)
    }
    updateSessionChrome()
  }
  createEffect(() => {
    const attachedSessionId = attachmentState()?.session_id ?? null
    if (attachedSessionId === hydratedPromptHistorySessionId) {
      return
    }
    hydratedPromptHistorySessionId = attachedSessionId
    restorePromptHistory(attachedSessionId)
    if (!attachedSessionId) {
      promptHistoryHydrationGeneration += 1
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
  const attachmentTokenKind = (kind: PromptAttachmentKind) => (kind === "image" ? "image" : kind === "pdf" ? "pdf" : "file")
  const hotkeySections = (): HotkeySection[] => [
    { title: "Global", items: GLOBAL_HOTKEYS },
    isAttached()
      ? { title: "Session", items: SESSION_HOTKEYS }
      : { title: "Waiting room", items: WAITING_ROOM_HOTKEYS },
  ]
  const sessionBrowserSessions = () => availableSessions()
    .filter((session) => session.status !== "Ended")
    .slice()
    .sort((left, right) => sessionBrowserSortTime(right) - sessionBrowserSortTime(left))
  const normalizeSessionBrowserIndex = () => {
    const sessions = sessionBrowserSessions()
    const index = Math.min(Math.max(0, sessionBrowserIndex()), Math.max(0, sessions.length - 1))
    if (index !== sessionBrowserIndex()) {
      setSessionBrowserIndex(index)
    }
    return index
  }
  const renderHotkeysOverlay = () => {
    if (!hotkeysOverlayBox) {
      return
    }
    for (const child of [...hotkeysOverlayBox.getChildren()]) {
      hotkeysOverlayBox.remove(child.id)
      child.destroyRecursively()
    }
    if (!hotkeysOpen() && !terminalPairingOpen() && !sessionBrowserOpen()) {
      hotkeysOverlayBox.requestRender()
      return
    }

    const scrim = new BoxRenderable(renderer, {
      position: "absolute",
      left: 0,
      top: 0,
      width: dimensions().width,
      height: dimensions().height,
      alignItems: "center",
      paddingTop: Math.max(1, Math.floor(dimensions().height / 5)),
      backgroundColor: RGBA.fromInts(0, 0, 0, 150),
    })
    scrim.onMouseUp = () => {
      if (sessionBrowserOpen()) {
        closeSessionBrowserDialog()
      } else if (terminalPairingOpen()) {
        closeTerminalPairingDialog()
      } else {
        closeHotkeys()
      }
    }

    if (sessionBrowserOpen()) {
      const sessions = sessionBrowserSessions()
      const panel = new BoxRenderable(renderer, {
        width: Math.min(112, Math.max(78, Math.floor(dimensions().width * 0.78))),
        maxWidth: dimensions().width - 4,
        backgroundColor: theme.backgroundPanel,
        paddingTop: 1,
        paddingBottom: 1,
        paddingLeft: 2,
        paddingRight: 2,
        flexDirection: "column",
        gap: 1,
      })
      panel.onMouseUp = (event) => {
        event.stopPropagation()
      }
      const header = new BoxRenderable(renderer, {
        flexDirection: "row",
        justifyContent: "space-between",
      })
      header.add(new TextRenderable(renderer, {
        content: "All Sessions",
        attributes: TextAttributes.BOLD,
        fg: theme.text,
      }))
      header.add(new TextRenderable(renderer, {
        content: "Enter opens • A/D confirm • Esc closes",
        fg: theme.textMuted,
      }))
      panel.add(header)
      if (sessions.length === 0) {
        panel.add(new TextRenderable(renderer, {
          content: "No sessions available.",
          fg: theme.textMuted,
        }))
      } else {
        const index = normalizeSessionBrowserIndex()
        const statusWidth = Math.max("Status".length, ...sessions.map((session) => sessionBrowserStatus(session).length))
        const lastUsedWidth = Math.max("Last used".length, "0000-00-00 00:00 UTC".length)
        const createdAtWidth = Math.max("Created at".length, "0000-00-00 00:00 UTC".length)
        panel.add(new TextRenderable(renderer, {
          content: `  ${"Session".padEnd(30, " ")} ${"Status".padEnd(statusWidth, " ")}  ${"Last used".padEnd(lastUsedWidth, " ")}  ${"Created at".padEnd(createdAtWidth, " ")}`,
          fg: theme.textMuted,
          wrapMode: "none",
        }))
        const maxRows = Math.max(4, Math.min(14, dimensions().height - 12))
        const start = Math.min(Math.max(0, index - maxRows + 1), Math.max(0, sessions.length - maxRows))
        for (const [offset, session] of sessions.slice(start, start + maxRows).entries()) {
          const rowIndex = start + offset
          const selected = rowIndex === index
          const title = sessionBrowserTitle(session)
          const content = `${selected ? ">" : " "} ${title.padEnd(30, " ")} ${sessionBrowserStatus(session).padEnd(statusWidth, " ")}  ${sessionBrowserTimestamp(session.last_used_at_ms ?? null).padEnd(lastUsedWidth, " ")}  ${sessionBrowserTimestamp(session.created_at_ms ?? null).padEnd(createdAtWidth, " ")}`
          panel.add(new TextRenderable(renderer, {
            content,
            fg: selected ? theme.primary : theme.text,
            ...(selected ? { attributes: TextAttributes.BOLD } : {}),
            wrapMode: "none",
          }))
        }
        if (sessions.length > maxRows) {
          panel.add(new TextRenderable(renderer, {
            content: `${start + 1}-${Math.min(sessions.length, start + maxRows)} of ${sessions.length}`,
            fg: theme.textMuted,
          }))
        }
      }
      scrim.add(panel)
      hotkeysOverlayBox.add(scrim)
      hotkeysOverlayBox.requestRender()
      return
    }

    if (terminalPairingOpen()) {
      const pairing = terminalPairingState()
      const panel = new BoxRenderable(renderer, {
        width: Math.min(96, Math.max(72, Math.floor(dimensions().width * 0.72))),
        maxWidth: dimensions().width - 4,
        backgroundColor: theme.backgroundPanel,
        paddingTop: 1,
        paddingBottom: 1,
        paddingLeft: 2,
        paddingRight: 2,
        flexDirection: "column",
        gap: 1,
      })
      panel.onMouseUp = (event) => {
        event.stopPropagation()
      }
      const header = new BoxRenderable(renderer, {
        flexDirection: "row",
        justifyContent: "space-between",
      })
      header.add(new TextRenderable(renderer, {
        content: "Add New Terminal",
        attributes: TextAttributes.BOLD,
        fg: theme.text,
      }))
      header.add(new TextRenderable(renderer, {
        content: "Esc closes",
        fg: theme.textMuted,
      }))
      panel.add(header)
      if (!pairing) {
        panel.add(new TextRenderable(renderer, {
          content: "Creating pairing link...",
          fg: theme.textMuted,
        }))
      } else {
        panel.add(new TextRenderable(renderer, {
          content: `Type: ${formatTerminalTypeLabel(pairing.terminal_type)}   Code: ${pairing.pairing_code}`,
          fg: theme.primary,
          attributes: TextAttributes.BOLD,
        }))
        panel.add(new TextRenderable(renderer, {
          content: `Expires: ${formatPairingExpiry(pairing.expires_at_ms)}`,
          fg: theme.textMuted,
        }))
        const qr = terminalPairingQrLines()
        if (qr.length > 0) {
          panel.add(new TextRenderable(renderer, {
            content: qr.join("\n"),
            fg: theme.text,
          }))
        }
        panel.add(new TextRenderable(renderer, {
          content: "Pairing link",
          fg: theme.primary,
          attributes: TextAttributes.BOLD,
        }))
        for (const line of wrapPairingLink(pairing.pairing_link, Math.max(36, Math.min(88, dimensions().width - 10)))) {
          panel.add(new TextRenderable(renderer, {
            content: line,
            fg: theme.text,
          }))
        }
      }
      scrim.add(panel)
      hotkeysOverlayBox.add(scrim)
      hotkeysOverlayBox.requestRender()
      return
    }

    const panel = new BoxRenderable(renderer, {
      width: HOTKEY_DIALOG_WIDTH,
      maxWidth: dimensions().width - 4,
      backgroundColor: theme.backgroundPanel,
      paddingTop: 1,
      paddingBottom: 1,
      paddingLeft: 2,
      paddingRight: 2,
      flexDirection: "column",
      gap: 1,
    })
    panel.onMouseUp = (event) => {
      event.stopPropagation()
    }

    const header = new BoxRenderable(renderer, {
      flexDirection: "row",
      justifyContent: "space-between",
    })
    header.add(new TextRenderable(renderer, {
      content: "Hotkeys",
      attributes: TextAttributes.BOLD,
      fg: theme.text,
    }))
    header.add(new TextRenderable(renderer, {
      content: "Esc closes",
      fg: theme.textMuted,
    }))
    panel.add(header)
    panel.add(new TextRenderable(renderer, {
      content: `${HOTKEY_TOGGLE_LABEL} toggles this list.`,
      fg: theme.textMuted,
    }))

    for (const section of hotkeySections()) {
      const sectionBox = new BoxRenderable(renderer, {
        flexDirection: "column",
        gap: 1,
      })
      sectionBox.add(new TextRenderable(renderer, {
        content: section.title,
        attributes: TextAttributes.BOLD,
        fg: theme.primary,
      }))
      for (const item of section.items) {
        const row = new BoxRenderable(renderer, {
          flexDirection: "row",
          gap: 2,
        })
        const keys = new BoxRenderable(renderer, {
          width: 22,
          flexShrink: 0,
        })
        keys.add(new TextRenderable(renderer, {
          content: item.keys,
          attributes: TextAttributes.BOLD,
          fg: theme.text,
        }))
        row.add(keys)
        row.add(new TextRenderable(renderer, {
          content: item.description,
          fg: theme.textMuted,
        }))
        sectionBox.add(row)
      }
      panel.add(sectionBox)
    }

    scrim.add(panel)
    hotkeysOverlayBox.add(scrim)
    hotkeysOverlayBox.requestRender()
  }
  const closeHotkeys = () => {
    if (!hotkeysOpen()) {
      return
    }
    const restoreTarget = hotkeysFocus
    hotkeyDebug(`close start open=${hotkeysOpen()} saved=${describeRenderableDebug(restoreTarget)?.type ?? "none"} current=${describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null)?.type ?? "none"}`)
    appLogger?.debug("closing hotkeys overlay", {
      hotkeys_open: hotkeysOpen(),
      restore_focus: describeRenderableDebug(restoreTarget),
      current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
    })
    setHotkeysOpen(false)
    renderHotkeysOverlay()
    startTimeout(() => {
      if (!restoreTarget || restoreTarget.isDestroyed) {
        hotkeyDebug(`close skip-restore saved=${describeRenderableDebug(restoreTarget)?.type ?? "none"}`)
        appLogger?.debug("hotkeys overlay skipped focus restore", {
          restore_focus: describeRenderableDebug(restoreTarget),
          current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
        })
        hotkeysFocus = null
        return
      }
      restoreTarget.focus()
      hotkeyDebug(`close restored saved=${describeRenderableDebug(restoreTarget)?.type ?? "none"} current=${describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null)?.type ?? "none"}`)
      appLogger?.debug("hotkeys overlay restored focus", {
        restore_focus: describeRenderableDebug(restoreTarget),
        current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
      })
      hotkeysFocus = null
    }, 1)
  }
  const closeTerminalPairingDialog = () => {
    if (!terminalPairingOpen()) {
      return
    }
    const restoreTarget = hotkeysFocus
    setTerminalPairingOpen(false)
    renderHotkeysOverlay()
    startTimeout(() => {
      if (restoreTarget && !restoreTarget.isDestroyed) {
        restoreTarget.focus()
      }
      hotkeysFocus = null
    }, 1)
  }
  const closeSessionBrowserDialog = () => {
    if (!sessionBrowserOpen()) {
      return
    }
    const restoreTarget = hotkeysFocus
    setSessionBrowserOpen(false)
    renderHotkeysOverlay()
    startTimeout(() => {
      if (restoreTarget && !restoreTarget.isDestroyed) {
        restoreTarget.focus()
      }
      hotkeysFocus = null
    }, 1)
  }
  const openHotkeys = () => {
    if (hotkeysOpen()) {
      return
    }
    const focused = (renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null
    hotkeysFocus = focused && !focused.isDestroyed
      ? focused
      : promptInput && !promptInput.isDestroyed
        ? promptInput
        : null
    hotkeyDebug(`open start current=${describeRenderableDebug(focused)?.type ?? "none"} saved=${describeRenderableDebug(hotkeysFocus)?.type ?? "none"}`)
    appLogger?.debug("opening hotkeys overlay", {
      hotkeys_open: hotkeysOpen(),
      current_focus: describeRenderableDebug(focused),
      saved_focus: describeRenderableDebug(hotkeysFocus),
    })
    hotkeysFocus?.blur()
    hotkeyDebug(`open blurred saved=${describeRenderableDebug(hotkeysFocus)?.type ?? "none"} current=${describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null)?.type ?? "none"}`)
    appLogger?.debug("hotkeys overlay blurred saved focus", {
      saved_focus: describeRenderableDebug(hotkeysFocus),
      current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
    })
    setHotkeysOpen(true)
    renderHotkeysOverlay()
    hotkeyDebug(`open done open=${hotkeysOpen()} saved=${describeRenderableDebug(hotkeysFocus)?.type ?? "none"}`)
    appLogger?.debug("hotkeys overlay opened", {
      hotkeys_open: hotkeysOpen(),
      saved_focus: describeRenderableDebug(hotkeysFocus),
      current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
    })
  }
  const openTerminalPairingDialog = async () => {
    if (terminalPairingOpen()) {
      return
    }
    const focused = (renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null
    hotkeysFocus = focused && !focused.isDestroyed
      ? focused
      : promptInput && !promptInput.isDestroyed
        ? promptInput
        : null
    hotkeysFocus?.blur()
    setTerminalPairingState(null)
    setTerminalPairingQrLines([])
    setHotkeysOpen(false)
    setTerminalPairingOpen(true)
    renderHotkeysOverlay()
    try {
      const pairing = await createTerminalPairingLink(client, "cli")
      const qrLines = await renderTerminalPairingQr(pairing.pairing_link)
      setTerminalPairingState(pairing)
      setTerminalPairingQrLines(qrLines)
      renderHotkeysOverlay()
      flashFooter("terminal pairing link created", "info")
    } catch (error) {
      closeTerminalPairingDialog()
      flashFooter(formatError(error), "error")
    }
  }
  const openSessionBrowserDialog = () => {
    if (sessionBrowserOpen()) {
      return
    }
    const sessions = sessionBrowserSessions()
    if (sessions.length === 0) {
      flashFooter("no sessions available to join", "error")
      return
    }
    const focused = (renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null
    hotkeysFocus = focused && !focused.isDestroyed
      ? focused
      : promptInput && !promptInput.isDestroyed
        ? promptInput
        : null
    hotkeysFocus?.blur()
    setHotkeysOpen(false)
    setTerminalPairingOpen(false)
    setSessionBrowserIndex(Math.min(Math.max(0, waitingRoomState().sessionIndex), sessions.length - 1))
    setSessionBrowserOpen(true)
    renderHotkeysOverlay()
    flashFooter("select a session to open, archive, or delete", "info")
  }
  const toggleHotkeys = () => {
    hotkeyDebug(`toggle open=${hotkeysOpen()} current=${describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null)?.type ?? "none"}`)
    appLogger?.debug("toggleHotkeys invoked", {
      hotkeys_open: hotkeysOpen(),
      saved_focus: describeRenderableDebug(hotkeysFocus),
      current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
    })
    if (hotkeysOpen()) {
      closeHotkeys()
      return
    }
    if (terminalPairingOpen()) {
      closeTerminalPairingDialog()
    }
    if (sessionBrowserOpen()) {
      closeSessionBrowserDialog()
    }
    openHotkeys()
  }
  const nextAttachmentToken = (kind: PromptAttachmentKind) => {
    const label = attachmentTokenKind(kind)
    const count = pendingAttachments().filter((file) => attachmentTokenKind(file.kind) === label).length + 1
    return `[${label} ${count}]`
  }
  const syncPendingPromptAttachmentsFromText = (value: string) => {
    setPendingAttachments((current) => current.filter((file) => value.includes(file.token)))
    refreshPromptAttachmentHighlights()
  }
  const insertPromptAttachmentTokens = (tokens: string[], at: number) => {
    if (tokens.length === 0) {
      return
    }
    const current = promptInput?.plainText ?? promptTextSnapshot
    const prefix = current.slice(0, at)
    const suffix = current.slice(at)
    const before = prefix && !/\s$/.test(prefix) ? " " : ""
    const after = suffix && !/^\s/.test(suffix) ? " " : ""
    const content = tokens.join(" ")
    const next = `${prefix}${before}${content}${after}${suffix}`
    setPromptText(next)
    if (promptInput) {
      promptInput.cursorOffset = prefix.length + before.length + content.length
    }
  }
  const removeLastPendingPromptAttachment = () => {
    const last = pendingAttachments().at(-1)
    if (!last) {
      return
    }
    removePromptAttachmentToken(last.token)
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }
  const addPendingPromptAttachments = (files: Array<Omit<PendingPromptAttachment, "token">>, at: number) => {
    const existing = new Set(pendingAttachments().map((file) => file.url))
    const next = files.filter((file) => !existing.has(file.url))
    if (next.length === 0) {
      return false
    }
    const counts = {
      image: pendingAttachments().filter((file) => attachmentTokenKind(file.kind) === "image").length,
      pdf: pendingAttachments().filter((file) => attachmentTokenKind(file.kind) === "pdf").length,
      file: pendingAttachments().filter((file) => attachmentTokenKind(file.kind) === "file").length,
    }
    const attachments = next.map((file) => {
      const kind = attachmentTokenKind(file.kind)
      counts[kind] += 1
      return { ...file, token: `[${kind} ${counts[kind]}]` }
    })
    setPendingAttachments((current) => [...current, ...attachments])
    insertPromptAttachmentTokens(attachments.map((file) => file.token), at)
    refreshPromptAttachmentHighlights()
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
    return true
  }
  const removePromptAttachmentToken = (token: string) => {
    const current = promptInput?.plainText ?? promptTextSnapshot
    const index = current.indexOf(token)
    if (index === -1) {
      setPendingAttachments((files) => files.filter((file) => file.token !== token))
      refreshPromptAttachmentHighlights()
      return
    }
    let start = index
    let end = index + token.length
    if (start > 0 && current[start - 1] === " " && (end === current.length || current[end] === " " || current[end] === "\n")) {
      start -= 1
    } else if (end < current.length && current[end] === " ") {
      end += 1
    }
    const next = `${current.slice(0, start)}${current.slice(end)}`
    setPendingAttachments((files) => files.filter((file) => file.token !== token))
    setPromptText(next)
    if (promptInput) {
      promptInput.cursorOffset = start
    }
  }
  const removePromptAttachmentsForEdit = (action: "backspace" | "delete") => {
    if (!promptInput) {
      return false
    }
    const text = promptInput.plainText
    const selection = promptInput.getSelection()
    const cursor = promptInput.cursorOffset
    const edit = resolvePromptAttachmentEdit(
      text,
      pendingAttachments().map((file) => file.token),
      action,
      cursor,
      selection,
    )
    if (!edit) {
      return false
    }
    if (edit.kind === "noop") {
      return true
    }
    if (edit.kind === "delete-text") {
      const hasSelection = selection && selection.start !== selection.end
      setPromptText(`${text.slice(0, edit.start)}${text.slice(edit.end)}`)
      promptInput.cursorOffset = hasSelection ? Math.min(selection.start, selection.end) : cursor
      updateSessionChrome()
      ;(renderer as { requestRender?: () => void }).requestRender?.()
      return true
    }
    setPendingAttachments((files) => files.filter((file) => !edit.tokens.includes(file.token)))
    setPromptText(`${text.slice(0, edit.start)}${text.slice(edit.end)}`)
    promptInput.cursorOffset = edit.start
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
    return true
  }
  const storePromptAttachment = async (file: ParsedPromptAttachment) => {
    const attachment = attachmentState()
    if (!attachment) {
      throw new Error("no active attachment available for storing prompt attachments")
    }
    const response = await client.send<Record<string, unknown>>(
      storeTransferredFileRequest(sessionState().id, attachment.id, file.path, file.filename),
    )
    const payload = expectVariant<{ result: StoredTransferArtifact }>(response, "FileTransferred")
    return {
      id: payload.result.artifact_id,
      url: pathToFileURL(payload.result.stored_path).href,
      mime: file.mime,
      filename: payload.result.display_name,
      kind: file.kind,
    }
  }
  const attachPromptFiles = async (files: ParsedPromptAttachment[], at = promptInput?.cursorOffset ?? promptTextSnapshot.length) => {
    const stored = []
    for (const file of files) {
      stored.push(await storePromptAttachment(file))
    }
    addPendingPromptAttachments(stored, at)
    if (files.length > 0) {
      flashFooter(`attached ${files.length} file${files.length === 1 ? "" : "s"}`, "info")
    }
  }
  const capturePromptScreenshot = async () => {
    const attachment = attachmentState()
    if (!attachment) {
      flashFooter("attach to a session before capturing screenshots", "error")
      return
    }
    const response = await client.send<Record<string, unknown>>(
      captureScreenshotRequest(sessionState().id, attachment.id),
    )
    const payload = expectVariant<{ result: CaptureScreenshotResult }>(response, "ScreenshotCaptured")
    if (payload.result.status !== "Captured" || !payload.result.artifact_path) {
      flashFooter(payload.result.message, "error")
      return
    }
    addPendingPromptAttachments([{
      id: `screenshot-${Date.now()}`,
      url: pathToFileURL(payload.result.artifact_path).href,
      mime: "image/png",
      filename: path.basename(payload.result.artifact_path),
      kind: "image",
    }], promptInput?.cursorOffset ?? promptTextSnapshot.length)
    flashFooter("attached screenshot", "info")
  }
  const handlePromptContentChange = () => {
    if (!promptInput) {
      return
    }
    const value = promptInput.plainText
    if (!isAttached()) {
      promptTextSnapshot = value
      syncCommandCenter(value)
      return
    }
    if (isProgrammaticPromptContentEcho({
      currentText: value,
      previousSnapshot: promptTextSnapshot,
      programmaticMutation: promptTextMuting,
      dropPending: promptDropPending,
    })) {
      promptTextSnapshot = value
      syncCommandCenter(value)
      return
    }
    if (promptHistoryIndex() !== null || promptHistoryDraft() !== null) {
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(value)
    }
    const drop = extractDroppedPromptAttachments(promptTextSnapshot, value, process.cwd())
    if (!drop) {
      syncPendingPromptAttachmentsFromText(value)
      promptTextSnapshot = value
      syncCommandCenter(value)
      const sessionId = attachmentState()?.session_id
      if (sessionId) {
        schedulePromptDraftPersist(sessionId, value)
      }
      return
    }
    setPromptText(drop.nextText)
    syncCommandCenter(drop.nextText)
    const sessionId = attachmentState()?.session_id
    if (sessionId) {
      schedulePromptDraftPersist(sessionId, drop.nextText)
    }
    promptDropPending = true
    void attachPromptFiles(drop.files, drop.insertAt)
      .catch((error) => {
        appLogger?.warn("prompt attachment drop failed", {
          error: formatError(error),
          paths: drop.files.map((file) => file.path),
        })
        flashFooter(`failed to attach files: ${formatError(error)}`, "error")
      })
      .finally(() => {
        promptDropPending = false
      })
  }
  const handleAttachmentCommand = async (commandLine: string) => {
    const value = commandLine.replace(/^\/attach\s*/, "").trim()
    if (!value) {
      flashFooter("usage: /attach <path...> | /attach clear | /attach screenshot", "error")
      return
    }
    if (value === "clear") {
      clearPendingPromptAttachments()
      flashFooter("cleared prompt attachments", "info")
      return
    }
    if (value === "screenshot") {
      await capturePromptScreenshot()
      return
    }
    const files = parsePromptAttachmentCommand(value, process.cwd())
    if (!files || files.length === 0) {
      flashFooter("drop or specify images, PDFs, or text files", "error")
      return
    }
    await attachPromptFiles(files)
  }

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
    pendingHistoryScrollRestore = 0
    const maxScrollTop = Math.max(0, transcriptScrollbox.scrollHeight - transcriptScrollbox.height)
    transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: maxScrollTop })
    transcriptScrollbox.requestRender()
    lastTranscriptScrollTop = transcriptScrollbox.scrollTop
  }

  const trackAgentFocusTransition = async <T,>(operation: () => Promise<T>): Promise<T> => {
    const transition = operation()
    const completion = transition.then(
      () => undefined,
      () => undefined,
    )
    pendingAgentFocusTransition = completion
    try {
      return await transition
    } finally {
      if (pendingAgentFocusTransition === completion) {
        pendingAgentFocusTransition = null
      }
    }
  }

  const waitForPendingAgentFocusTransition = async () => {
    if (!pendingAgentFocusTransition) {
      return
    }
    await pendingAgentFocusTransition
  }

  const appendUserPrompt = (text: string, agentId?: string | null) => {
    recordTurnActivity("prompt_submit")
    turnCompletionConfirmed = false
    cancelPendingTurnCompletion()
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

  const flashFooter = (message: string, tone: FooterFlash["tone"]) => {
    if (footerFlashTimeout) {
      clearTimeout(footerFlashTimeout)
    }
    setFooterFlash({ message, tone })
    updateSessionChrome()
    footerFlashTimeout = startTimeout(() => {
      footerFlashTimeout = undefined
      setFooterFlash(null)
      updateSessionChrome()
    }, 10_000)
  }

  const hotkeyDebug = (message: string) => {
    appLogger?.debug("hotkeys footer debug", { detail: message })
    if (!DEBUG_LOGS_ENABLED) {
      return
    }
    flashFooter(`[hotkeys] ${message}`, "info")
  }

  const copyTextWithFeedback = (text: string | null | undefined) => {
    if (!text) {
      return false
    }
    void copyTextToClipboard(text, renderer)
      .then(() => {
        flashFooter("selection copied to clipboard", "info")
      })
      .catch((error) => {
        appLogger?.warn("selection copy failed", {
          error: formatError(error),
        })
        flashFooter("failed to copy selection", "error")
      })
    return true
  }

  const copyPromptSelection = () => {
    const selection = promptInput?.getSelection()
    if (!selection || selection.start === selection.end || !promptInput) {
      return false
    }
    const start = Math.max(0, Math.min(selection.start, selection.end))
    const end = Math.min(promptInput.plainText.length, Math.max(selection.start, selection.end))
    return copyTextWithFeedback(promptInput.plainText.slice(start, end))
  }

  const copySelection = () => {
    const text = renderer.getSelection()?.getSelectedText()
    renderer.clearSelection()
    copyTextWithFeedback(text)
  }

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
      turnCompletionConfirmed = false
      cancelPendingTurnCompletion()
    } else if (turnCompletionConfirmed || shouldConfirmIdleCompletion) {
      turnCompletionConfirmed = true
      scheduleTurnCompletion()
    } else {
      cancelPendingTurnCompletion()
    }
    setProviderActivityLabel(transition.nextFocusedActivityLabel)
    setActiveStatusLabel(transition.nextFocusedActivityLabel)
    if (promptLifecycle.activePromptChanged) {
      setSubmitting(false)
      submittingAgentId = null
      stopRequestInFlight = false
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
        turnCompletionConfirmed = true
        cancelPendingTurnCompletion()
        setWorking(false)
      }
    }
    if (!transition.nextHasPromptWork) {
      setSubmitting(false)
      stopRequestInFlight = false
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
      turnCompletionConfirmed = false
      cancelPendingTurnCompletion()
      activeToolLabels.clear()
      setAgentActivityLabels({})
      setStreamingAgentId(null)
      setSubmitting(false)
      submittingAgentId = null
      stopRequestInFlight = false
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
      cancelPendingTurnCompletion()
      setWorking(true)
    } else if (turnCompletionConfirmed) {
      scheduleTurnCompletion()
    }
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
    turnCompletionConfirmed = true
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

  const flushPendingTerminalRecords = () => {
    if (pendingTerminalRecordFlush) {
      clearTimeout(pendingTerminalRecordFlush)
      pendingTerminalRecordFlush = undefined
    }
    if (pendingTerminalRecords.length === 0) {
      return
    }
    const records = pendingTerminalRecords
    pendingTerminalRecords = []
    runUiBatch(() => {
      for (const record of records) {
        processTerminalOutputRecord(record)
      }
    })
  }

  const queueTerminalOutputRecords = (records: TerminalOutputRecord[]) => {
    if (records.length === 0) {
      return
    }
    pendingTerminalRecords.push(...records)
    if (pendingTerminalRecordFlush) {
      return
    }
    pendingTerminalRecordFlush = startTimeout(() => {
      pendingTerminalRecordFlush = undefined
      flushPendingTerminalRecords()
    }, STREAM_BATCH_WINDOW_MS)
  }

  const ensureChromeRenderables = () => {
    if (promptStateBox && !promptStateText) {
      promptStateText = new TextRenderable(renderer, { fg: theme.textMuted, wrapMode: "none" })
      promptStateBox.add(promptStateText)
    }
    if (footerSummaryBox && !footerSummaryText) {
      footerSummaryText = new TextRenderable(renderer, { fg: theme.textMuted, wrapMode: "none" })
      footerFlashText = new TextRenderable(renderer, { fg: theme.info, wrapMode: "none" })
      footerSummaryBox.add(footerSummaryText)
      footerSummaryBox.add(footerFlashText)
    }
    if (statusIndicatorBox && !statusOpenText) {
      statusOpenText = new TextRenderable(renderer, { content: "", fg: theme.textMuted, wrapMode: "none" })
      statusCloseText = new TextRenderable(renderer, { content: "", fg: theme.textMuted, wrapMode: "none" })
      statusIndicatorBox.add(statusOpenText)
      statusLabelTexts = Array.from({ length: STATUS_BADGE_WIDTH }, () => {
        const text = new TextRenderable(renderer, { wrapMode: "none" })
        statusIndicatorBox!.add(text)
        return text
      })
      statusIndicatorBox.add(statusCloseText)
    }
  }

  const ensureSplitPaneFooterRenderables = (
    footerBox: BoxRenderable | undefined,
    badgeTexts: TextRenderable[],
    parts: SplitPaneFooterTextGroup,
    assignParts: (value: SplitPaneFooterTextGroup) => void,
    assignBadgeTexts: (value: TextRenderable[]) => void,
  ) => {
    if (!footerBox || parts.agentText) {
      return
    }
    footerBox.flexDirection = "row"
    footerBox.gap = 1
    const badgeBox = new BoxRenderable(renderer, {
      flexDirection: "row",
      flexShrink: 0,
    })
    const infoBox = new BoxRenderable(renderer, {
      flexDirection: "row",
      flexShrink: 0,
    })
    const nextBadgeTexts = Array.from({ length: STATUS_BADGE_WIDTH }, () => new TextRenderable(renderer, { wrapMode: "none" }))
    for (const text of nextBadgeTexts) {
      badgeBox.add(text)
    }
    const nextParts: SplitPaneFooterTextGroup = {
      agentText: new TextRenderable(renderer, { wrapMode: "none" }),
      agentDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
      providerText: new TextRenderable(renderer, { wrapMode: "none" }),
      providerDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
      modelText: new TextRenderable(renderer, { wrapMode: "none" }),
      modelDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
      variantText: new TextRenderable(renderer, { wrapMode: "none" }),
      variantDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
      modeText: new TextRenderable(renderer, { wrapMode: "none" }),
      modeDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
      permissionText: new TextRenderable(renderer, { wrapMode: "none" }),
    }
    infoBox.add(nextParts.agentText)
    infoBox.add(nextParts.agentDividerText)
    infoBox.add(nextParts.providerText)
    infoBox.add(nextParts.providerDividerText)
    infoBox.add(nextParts.modelText)
    infoBox.add(nextParts.modelDividerText)
    infoBox.add(nextParts.variantText)
    infoBox.add(nextParts.variantDividerText)
    infoBox.add(nextParts.modeText)
    infoBox.add(nextParts.modeDividerText)
    infoBox.add(nextParts.permissionText)
    footerBox.add(badgeBox)
    footerBox.add(infoBox)
    assignParts(nextParts)
    assignBadgeTexts(nextBadgeTexts)
  }

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

  const renderStatusBadgeTexts = (
    texts: TextRenderable[],
    label: string,
    tone: StatusBadgeTone,
  ) => {
    renderStatusBadgeParts(texts, [{ label, tone }], STATUS_BADGE_WIDTH)
  }

  const renderStatusBadgeParts = (
    texts: TextRenderable[],
    parts: StatusBadgePart[],
    minWidth = 0,
  ) => {
    const cells = badgeCells(parts)
    const width = Math.max(minWidth, cells.length)
    for (let index = 0; index < texts.length; index += 1) {
      const cell = index < width ? cells[index] : undefined
      const character = cell?.character ?? " "
      const tone = cell?.tone ?? "idle"
      let fg = theme.success
      if (tone === "disconnected" || tone === "error") {
        fg = theme.error
      } else if (tone === "working") {
        const distance = reflectedDistance(cell?.partIndex ?? 0, cell?.partLength ?? 0, workingAnimationFrame())
        fg = distance === 0 ? theme.primary : distance === 1 ? theme.warning : theme.secondary
      }
      setTextRenderable(
        texts[index],
        character,
        fg,
        tone === "working" && character.trim() ? TextAttributes.BOLD : TextAttributes.NONE,
      )
    }
  }

  const badgeCells = (parts: StatusBadgePart[]) => {
    const cells: Array<{
      character: string
      tone: StatusBadgeTone
      partIndex: number
      partLength: number
    }> = []
    for (const [partOffset, part] of parts.entries()) {
      if (partOffset > 0) {
        cells.push({
          character: " ",
          tone: "idle",
          partIndex: 0,
          partLength: 0,
        })
      }
      for (let index = 0; index < part.label.length; index += 1) {
        cells.push({
          character: part.label[index] ?? " ",
          tone: part.tone,
          partIndex: index,
          partLength: part.label.length,
        })
      }
    }
    return cells
  }

  const ensureStatusLabelTextCount = (count: number) => {
    if (!statusIndicatorBox) {
      return
    }
    const moveCloseText = Boolean(statusCloseText && statusLabelTexts.length < count)
    if (statusCloseText && moveCloseText) {
      statusIndicatorBox.remove(statusCloseText.id)
    }
    while (statusLabelTexts.length < count) {
      const text = new TextRenderable(renderer, { wrapMode: "none" })
      statusLabelTexts.push(text)
      statusIndicatorBox.add(text)
    }
    if (statusCloseText && moveCloseText) {
      statusIndicatorBox.add(statusCloseText)
    }
  }

  const renderSplitPaneFooters = () => {
    const showAgentFooters = isAttached() && !workflowScreenActive() && responseVisibleAgents().length > 0
    ensureSplitPaneFooterRenderables(
      responsePrimaryFooterBox,
      responsePrimaryFooterBadgeTexts,
      responsePrimaryFooterParts,
      (value) => {
        responsePrimaryFooterParts = value
      },
      (value) => {
        responsePrimaryFooterBadgeTexts = value
      },
    )
    for (let slotIndex = 0; slotIndex < maxAgentsPerScreen() - 1; slotIndex += 1) {
      ensureSplitPaneFooterRenderables(
        responseAuxiliaryFooterBoxes[slotIndex],
        responseAuxiliaryFooterBadgeTexts[slotIndex] ?? [],
        responseAuxiliaryFooterParts[slotIndex] ?? {},
        (value) => {
          responseAuxiliaryFooterParts[slotIndex] = value
        },
        (value) => {
          responseAuxiliaryFooterBadgeTexts[slotIndex] = value
        },
      )
    }

    if (!showAgentFooters) {
      renderStatusBadgeTexts(responsePrimaryFooterBadgeTexts, "", "idle")
      setTextRenderable(responsePrimaryFooterParts.agentText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.agentDividerText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.providerText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.providerDividerText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.modelText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.modelDividerText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.variantText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.variantDividerText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.modeText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.modeDividerText, "", theme.textMuted)
      setTextRenderable(responsePrimaryFooterParts.permissionText, "", theme.textMuted)
      responsePrimaryFooterBox?.requestRender()
      for (let slotIndex = 0; slotIndex < maxAgentsPerScreen() - 1; slotIndex += 1) {
        renderStatusBadgeTexts(responseAuxiliaryFooterBadgeTexts[slotIndex] ?? [], "", "idle")
        const parts = responseAuxiliaryFooterParts[slotIndex]
        setTextRenderable(parts?.agentText, "", theme.textMuted)
        setTextRenderable(parts?.agentDividerText, "", theme.textMuted)
        setTextRenderable(parts?.providerText, "", theme.textMuted)
        setTextRenderable(parts?.providerDividerText, "", theme.textMuted)
        setTextRenderable(parts?.modelText, "", theme.textMuted)
        setTextRenderable(parts?.modelDividerText, "", theme.textMuted)
        setTextRenderable(parts?.variantText, "", theme.textMuted)
        setTextRenderable(parts?.variantDividerText, "", theme.textMuted)
        setTextRenderable(parts?.modeText, "", theme.textMuted)
        setTextRenderable(parts?.modeDividerText, "", theme.textMuted)
        setTextRenderable(parts?.permissionText, "", theme.textMuted)
        responseAuxiliaryFooterBoxes[slotIndex]?.requestRender()
      }
      return
    }

    const providerRun = providerRunState()
    const visibleAgents = responseVisibleAgents()
    const renderFooter = (
      agent: AgentInstance | null | undefined,
      footerBox: BoxRenderable | undefined,
      parts: SplitPaneFooterTextGroup | undefined,
      badgeTexts: TextRenderable[],
    ) => {
      const selectionOverride = agent?.id === focusedAgentId()
        ? currentProviderSelection()
        : null
      const badge = agentPaneStatusBadge(
        agent ?? null,
        agent ? agentActivityLabels()[agent.id] ?? null : null,
        agent ? hasPromptWorkByAgent()[agent.id] ?? false : false,
        agent?.id === streamingAgentId(),
        agent ? agentBusyLatch(agent.id) : false,
      )
      const focused = agent?.id === focusedAgentId()
      renderStatusBadgeTexts(badgeTexts, badge.label, badge.tone)
      const activeRun = providerRun && providerRun.agent_instance_id === agent?.id
        ? {
            agentInstanceId: providerRun.agent_instance_id,
            model: providerRun.model,
            variant: providerRun.variant,
          }
        : null
      const nextParts = formatSplitPaneFooterParts(
        agent
          ? {
              ...agent,
              execution_mode: agent.execution_mode_override
                ?? ((sessionState().config_state?.values?.["agents.mode"] as "build" | "plan" | undefined) ?? "build"),
              permission_level: agent.permission_level_override
                ?? ((sessionState().config_state?.values?.["agents.permissions"] as "required" | "yolo" | undefined) ?? "yolo"),
              location_label: agentLocationLabel(agent),
            }
          : null,
        activeRun,
        null,
        selectionOverride
          ? { model: selectionOverride.model, variant: selectionOverride.effort }
          : undefined,
      )
      const partTones = nextParts.map((part) => part.tone)
      const partTexts = nextParts.map((part) => part.text)
      const setPart = (
        text: TextRenderable | undefined,
        content: string,
        tone: PromptMetaTone | undefined,
        bold = false,
      ) => {
        setTextRenderable(
          text,
          content,
          tone ? promptMetaToneColor(tone) : theme.textMuted,
          bold ? TextAttributes.BOLD : TextAttributes.NONE,
        )
      }
      setPart(parts?.agentText, partTexts[0] ?? "", partTones[0], focused)
      setTextRenderable(parts?.agentDividerText, partTexts[1] ? " • " : "", theme.textMuted)
      setPart(parts?.providerText, partTexts[1] ?? "", partTones[1], focused)
      setTextRenderable(parts?.providerDividerText, partTexts[2] ? " • " : "", theme.textMuted)
      setPart(parts?.modelText, partTexts[2] ?? "", partTones[2], focused)
      setTextRenderable(parts?.modelDividerText, partTexts[3] ? " • " : "", theme.textMuted)
      setPart(parts?.variantText, partTexts[3] ?? "", partTones[3], focused)
      setTextRenderable(parts?.variantDividerText, partTexts[4] ? " • " : "", theme.textMuted)
      setPart(parts?.modeText, partTexts[4] ?? "", partTones[4], focused)
      setTextRenderable(parts?.modeDividerText, partTexts[5] ? " • " : "", theme.textMuted)
      setPart(parts?.permissionText, partTexts[5] ?? "", partTones[5], focused)
      footerBox?.requestRender()
    }

    renderFooter(visibleAgents[0] ?? null, responsePrimaryFooterBox, responsePrimaryFooterParts, responsePrimaryFooterBadgeTexts)
    for (let slotIndex = 0; slotIndex < maxAgentsPerScreen() - 1; slotIndex += 1) {
      renderFooter(
        visibleAgents[slotIndex + 1] ?? null,
        responseAuxiliaryFooterBoxes[slotIndex],
        responseAuxiliaryFooterParts[slotIndex],
        responseAuxiliaryFooterBadgeTexts[slotIndex] ?? [],
      )
    }
    responsePrimaryFooterBox?.requestRender()
  }

  const renderInteractionChoices = (
    container: BoxRenderable,
    interaction: RuntimeInteraction,
    focused: boolean,
  ) => {
    const choiceCount = interaction.choices.length + (interaction.custom_choice ? 1 : 0)
    const selectedIndex = Math.min(
      interactionChoiceSelection.get(interaction.id) ?? 0,
      Math.max(0, choiceCount - 1),
    )
    interactionChoiceSelection.set(interaction.id, selectedIndex)
    const choicesBox = new BoxRenderable(renderer, {
      flexDirection: "row",
      gap: 1,
      flexShrink: 0,
    })
    interaction.choices.forEach((choice, index) => {
      const text = new TextRenderable(renderer, { wrapMode: "none" })
      const selected = focused && index === selectedIndex
      const tone = choice.style === "danger"
        ? theme.error
        : choice.style === "secondary"
          ? theme.textMuted
          : theme.primary
      text.content = `${selected ? ">" : " "} ${index + 1}.${choice.label}`
      text.fg = selected ? theme.background : tone
      text.bg = selected ? tone : undefined
      text.attributes = selected ? TextAttributes.BOLD : TextAttributes.NONE
      choicesBox.add(text)
    })
    if (interaction.custom_choice) {
      const index = interaction.choices.length
      const text = new TextRenderable(renderer, { wrapMode: "none" })
      const selected = focused && index === selectedIndex
      const editing = interactionCustomEditing.has(interaction.id)
      const value = interactionCustomReplies.get(interaction.id) ?? ""
      const placeholder = interaction.custom_choice.placeholder ?? "type another option"
      const renderedValue = value ? value : `<${placeholder}>`
      text.content = `${selected ? ">" : " "} ${index + 1}.${interaction.custom_choice.label}: ${renderedValue}${editing ? "_" : ""}`
      text.fg = selected ? theme.background : theme.primary
      text.bg = selected ? theme.primary : undefined
      text.attributes = selected ? TextAttributes.BOLD : TextAttributes.NONE
      choicesBox.add(text)
    }
    container.add(choicesBox)
  }

  const renderInteractionStrip = (
    box: BoxRenderable | undefined,
    agent: AgentInstance | null | undefined,
  ) => {
    if (!box) {
      return
    }
    for (const child of [...box.getChildren()]) {
      box.remove(String(child.id))
      child.destroyRecursively?.()
    }
    const interaction = activeInteractionForAgent(agent?.id ?? null)
    box.visible = Boolean(interaction)
    box.flexDirection = "column"
    box.gap = 0
    box.paddingLeft = interaction ? 1 : 0
    box.paddingRight = interaction ? 1 : 0
    box.paddingTop = interaction ? 0 : 0
    box.paddingBottom = interaction ? 0 : 0
    box.backgroundColor = theme.backgroundElement
    if (!interaction) {
      box.requestRender?.()
      return
    }

    const focused = agent?.id === focusedAgentId()
    const titleLine = new TextRenderable(renderer, {
      wrapMode: "char",
      fg: interaction.level === "critical"
        ? theme.error
        : interaction.level === "warning"
          ? theme.warning
          : theme.info,
      attributes: TextAttributes.BOLD,
    })
    const titlePrefix = interaction.level.toUpperCase()
    titleLine.content = interaction.title
      ? `${titlePrefix} • ${interaction.title}`
      : titlePrefix
    const messageLine = new TextRenderable(renderer, {
      wrapMode: "word",
      fg: theme.text,
    })
    const timeoutSuffix = interaction.timeout_sec
      ? ` • timeout ${interaction.timeout_sec}s`
      : ""
    messageLine.content = `${interaction.message}${timeoutSuffix}`
    box.add(titleLine)
    box.add(messageLine)
    renderInteractionChoices(box, interaction, focused)
    box.requestRender?.()
  }

  const renderAgentInteractions = () => {
    const visibleAgents = responseVisibleAgents()
    renderInteractionStrip(responsePrimaryInteractionBox, visibleAgents[0] ?? null)
    for (let slotIndex = 0; slotIndex < maxAgentsPerScreen() - 1; slotIndex += 1) {
      renderInteractionStrip(
        responseAuxiliaryInteractionBoxes[slotIndex],
        visibleAgents[slotIndex + 1] ?? null,
      )
    }
  }

  const promptMetaToneColor = (tone: PromptMetaTone) => theme[tone]

  const setPromptMetaRenderables = (parts: PromptMetaPart[]) => {
    const usage = promptUsageMeta()
    const renderUsageMeta = () => {
      setTextRenderable(promptMetaUsageDividerText, "", theme.textMuted)
      setTextRenderable(
        promptMetaUsageTokensText,
        usage?.tokensLabel ?? "",
        usage ? theme.secondary : theme.textMuted,
        usage ? TextAttributes.BOLD : TextAttributes.NONE,
      )
      setTextRenderable(promptMetaUsageBarOpenText, usage?.usageLabel ? " [" : "", theme.textMuted)
      setTextRenderable(
        promptMetaUsageBarFilledText,
        usage?.barFilled ?? "",
        theme.primary,
        usage?.barFilled ? TextAttributes.BOLD : TextAttributes.NONE,
      )
      setTextRenderable(promptMetaUsageBarEmptyText, usage?.usageLabel ? (usage?.barEmpty ?? "") : "", theme.textMuted)
      setTextRenderable(promptMetaUsageBarCloseText, usage?.usageLabel ? "]" : "", theme.textMuted)
      setTextRenderable(
        promptMetaUsagePercentText,
        usage?.usageLabel ? ` ${usage.usageLabel}` : "",
        usage?.usageLabel ? theme.info : theme.textMuted,
        usage?.usageLabel ? TextAttributes.BOLD : TextAttributes.NONE,
      )
    }

    if (parts.length === 0) {
      setTextRenderable(promptMetaProviderText, "", theme.textMuted)
      setTextRenderable(promptMetaProviderDividerText, "", theme.textMuted)
      setTextRenderable(promptMetaModelText, "", theme.textMuted)
      setTextRenderable(promptMetaModelDividerText, "", theme.textMuted)
      setTextRenderable(promptMetaVariantText, "", theme.textMuted)
      renderUsageMeta()
      return
    }

    setTextRenderable(promptMetaProviderText, "", theme.textMuted)
    setTextRenderable(promptMetaProviderDividerText, "", theme.textMuted)
    setTextRenderable(promptMetaModelText, "", theme.textMuted)
    setTextRenderable(promptMetaModelDividerText, "", theme.textMuted)
    setTextRenderable(promptMetaVariantText, "", theme.textMuted)
    renderUsageMeta()
  }

  const renderHistoryLoadingIndicator = () => {
    if (!historyLoadingBox) {
      return
    }
    historyLoadingBox.visible = loadingHistory()
    if (loadingHistory()) {
      if (!historyLoadingText) {
        historyLoadingText = new TextRenderable(renderer, {
          content: "loading...",
          fg: theme.textMuted,
          wrapMode: "none",
        })
        historyLoadingBox.add(historyLoadingText)
      }
    } else if (historyLoadingText) {
      historyLoadingBox.remove(historyLoadingText.id)
      historyLoadingText.destroyRecursively()
      historyLoadingText = undefined
    }
    historyLoadingBox.requestRender()
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
    if (uiBatchDepth > 0) {
      pendingTranscriptRender = true
      return
    }
    renderScheduler.requestRenderable(transcriptScrollbox)
  }

  const flushScheduledSessionChromeUpdate = () => {
    if (pendingSessionChromeFlush) {
      clearTimeout(pendingSessionChromeFlush)
      pendingSessionChromeFlush = undefined
    }
    applySessionChromeUpdate()
  }

  const renderSessionChromeBoundary = () => {
    flushScheduledSessionChromeUpdate()
  }

  const flushDeferredUiUpdates = () => {
    if (pendingTranscriptRender) {
      pendingTranscriptRender = false
      renderScheduler.requestRenderable(transcriptScrollbox)
    }
    if (pendingSessionChromeUpdate) {
      pendingSessionChromeUpdate = false
      flushScheduledSessionChromeUpdate()
    }
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
    ensureChromeRenderables()
    if (!isAttached()) {
      lastLoggedFocusedBadgeState = null
      setTextRenderable(statusOpenText, "", theme.textMuted)
      ensureStatusLabelTextCount(STATUS_BADGE_WIDTH)
      renderStatusBadgeParts(statusLabelTexts, [], STATUS_BADGE_WIDTH)
      setTextRenderable(statusCloseText, "", theme.textMuted)
      statusIndicatorBox?.requestRender()
      return
    }
    const badge = focusedStatusBadge()
    logFocusedBadgeChange(badge)
    setTextRenderable(statusOpenText, "", theme.textMuted)
    ensureStatusLabelTextCount(Math.max(STATUS_BADGE_WIDTH, badge.label.length))
    renderStatusBadgeParts(statusLabelTexts, badge.parts, Math.max(STATUS_BADGE_WIDTH, badge.label.length))
    setTextRenderable(statusCloseText, "", theme.textMuted)
    statusIndicatorBox?.requestRender()
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

  const refreshSplitPaneFocusRepaint = () => {
    const refreshToken = ++pendingSplitPaneRefresh
    const refresh = () => {
      if (refreshToken !== pendingSplitPaneRefresh) {
        return
      }
      applyResponseLayout()
      scheduleResponsePaneRepaint()
    }

    refresh()
    startTimeout(refresh, 0)
  }

  const applySessionChromeUpdate = () => {
    if (uiBatchDepth > 0) {
      pendingSessionChromeUpdate = true
      return
    }
    ensureChromeRenderables()
    syncPromptPlaceholder()
    setTextRenderable(
      promptStateText,
      fatalError() ? "error" : submitting() ? "thinking" : footerHint(),
      fatalError() ? theme.error : submitting() ? theme.primary : theme.textMuted,
    )
    setPromptMetaRenderables(isAttached() ? promptMetaParts() : [])
    promptStateBox?.requestRender()
    setTextRenderable(
      footerSummaryText,
      isAttached()
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
      theme.textMuted,
    )
    setTextRenderable(
      footerFlashText,
      footerFlash() ? ` • ${footerFlash()!.message}` : "",
      footerFlash()?.tone === "error" ? theme.error : theme.info,
      footerFlash() ? TextAttributes.BOLD : TextAttributes.NONE,
    )
    footerSummaryBox?.requestRender()
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

  const updateSessionChrome = () => {
    if (uiBatchDepth > 0) {
      pendingSessionChromeUpdate = true
      return
    }
    if (!shouldThrottleSessionChrome()) {
      flushScheduledSessionChromeUpdate()
      return
    }
    if (pendingSessionChromeFlush) {
      return
    }
    pendingSessionChromeFlush = startTimeout(() => {
      pendingSessionChromeFlush = undefined
      applySessionChromeUpdate()
    }, CHROME_UPDATE_THROTTLE_MS)
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
      const restoreToken = ++pendingHistoryScrollRestore
      await new Promise<void>((resolve) => {
        const restoreScroll = (remainingAttempts: number, lastHeight = -1, stableFrames = 0) => {
          if (!transcriptScrollbox || scrollbox !== transcriptScrollbox || restoreToken !== pendingHistoryScrollRestore) {
            pendingHistoryScrollRestore = 0
            resolve()
            return
          }

          const nextScrollTop = computePrependedHistoryScrollTop(
            previousScrollTop,
            previousScrollHeight,
            scrollbox.scrollHeight,
            previousViewportHeight,
          )
          scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: nextScrollTop })
          scrollbox.requestRender()
          lastTranscriptScrollTop = scrollbox.scrollTop

          const closeEnough = Math.abs(scrollbox.scrollTop - nextScrollTop) <= 1
          const nextStableFrames = scrollbox.scrollHeight === lastHeight ? stableFrames + 1 : 0
          if ((closeEnough && nextStableFrames >= 1) || remainingAttempts <= 1) {
            pendingHistoryScrollRestore = 0
            resolve()
            return
          }

          startTimeout(() => restoreScroll(remainingAttempts - 1, scrollbox.scrollHeight, nextStableFrames), 16)
        }

        scrollbox.requestRender()
        startTimeout(() => restoreScroll(10), 0)
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
    const promptHistoryGeneration = ++promptHistoryHydrationGeneration
    const visibleAgentId = selectResponsePaneAgents(
      session.agents,
      session.focused_agent_id,
      splitAgentResponseMode(),
      maxAgentsPerScreen(),
    ).visibleTranscriptAgentId

    if (!visibleAgentId) {
      replaceTranscriptEntries([], null)
      setNextHistoryCursor(null)
      await loadAndApplyPromptHistoryFromSession(session.id, promptHistoryGeneration)
      return
    }

    const historyPage = await getSessionHistory(client, session.id, null, visibleAgentId)
    await loadAndApplyPromptHistoryFromSession(session.id, promptHistoryGeneration)
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
      stopRequestInFlight = false
    },
    bumpHistoryLoadGeneration: () => {
      historyLoadGeneration += 1
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
    detachAttachment: (attachmentId) => client.send(detachFromSessionRequest(attachmentId)).then(() => {}),
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

  const loadOlderHistoryPage = async () => {
    if (!isAttached() || loadingHistory() || nextHistoryCursor() === null) {
      return
    }

    setHistoryLoadingState(true)
    const generation = historyLoadGeneration
    const sessionId = sessionState().id
    const cursor = nextHistoryCursor()
    const agentId = visibleTranscriptAgentId()
    try {
      let historyPage = await getSessionHistory(client, sessionId, cursor, agentId)
      let hydratedEntries = hydrateTranscriptEntries(historyPage.entries)
      while (hydratedEntries.length > 0 && hydratedEntries[0]?.role !== "user" && historyPage.next_cursor !== null) {
        historyPage = await getSessionHistory(client, sessionId, historyPage.next_cursor, agentId)
        hydratedEntries = [...hydrateTranscriptEntries(historyPage.entries), ...hydratedEntries]
      }
      if (generation !== historyLoadGeneration || !isAttached() || sessionState().id !== sessionId) {
        return
      }
      const nextEntries = reindexTranscriptEntries(hydratedEntries, entryCounter())
      await prependTranscriptEntries(nextEntries)
      setNextHistoryCursor(historyPage.next_cursor)
      scheduleShortViewportHistoryCheck()
    } catch (error) {
      appLogger?.warn("older history load failed", {
        error: formatError(error),
      })
      flashFooter("failed to load older history", "error")
    } finally {
      setHistoryLoadingState(false)
    }
  }

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
    createSessionInvite: async (sessionId, expiresInMs, maxUses) => {
      const response = await client.send<Record<string, unknown>>(
        createSessionInviteRequest(sessionId, expiresInMs, maxUses),
      )
      return expectVariant<{
        invite: { invite_token: string; invite: { invite_id: string } }
        session: RuntimeSession
      }>(response, "SessionInviteCreated")
    },
    joinSessionInvite: async (inviteToken, userId) => {
      const response = await client.send<Record<string, unknown>>(
        joinSessionInviteRequest(inviteToken, userId),
      )
      return expectVariant<{ member: { user_id: string }; session: RuntimeSession }>(
        response,
        "SessionInviteJoined",
      )
    },
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
    createCloudSessionInvite: async (sessionId, inviteOptions) => {
      const response = await client.send<Record<string, unknown>>(
        createCloudSessionInviteRequest(sessionId, inviteOptions),
      )
      return expectVariant<Record<string, unknown>>(response, "CloudSessionInviteCreated")
    },
    acceptCloudSessionInvite: async (inviteToken) => {
      const response = await client.send<Record<string, unknown>>(
        acceptCloudSessionInviteRequest(inviteToken),
      )
      return expectVariant<Record<string, unknown>>(response, "CloudSessionInviteAccepted")
    },
    listCloudSessionMembers: async (sessionId) => {
      const response = await client.send<Record<string, unknown>>(
        listCloudSessionMembersRequest(sessionId),
      )
      return expectVariant<Record<string, unknown>>(response, "CloudSessionMembersListed")
    },
    listCloudCollaborators: async () => {
      const response = await client.send<Record<string, unknown>>(listCloudCollaboratorsRequest())
      return expectVariant<{ collaborators: Record<string, unknown>[] }>(
        response,
        "CloudCollaboratorsListed",
      ).collaborators
    },
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
    createWorkspaceLink: async (name) => {
      const response = await client.send<Record<string, unknown>>(
        createWorkspaceLinkRequest(sessionState().id, name),
      )
      return expectVariant(response, "WorkspaceLinkCreated")
    },
    listWorkspaceLinks: async () => {
      const response = await client.send<Record<string, unknown>>(
        listWorkspaceLinksRequest(sessionState().id),
      )
      return expectVariant<{ links: WorkspaceLinkDefinition[] }>(response, "WorkspaceLinksListed").links
    },
    showWorkspaceLink: async (linkRef) => {
      const response = await client.send<Record<string, unknown>>(
        showWorkspaceLinkRequest(sessionState().id, linkRef),
      )
      return expectVariant<{ link: WorkspaceLinkDefinition }>(response, "WorkspaceLinkShown").link
    },
    attachWorkspaceLink: async (linkRef, repoRoot) => {
      const response = await client.send<Record<string, unknown>>(
        attachWorkspaceLinkRequest(sessionState().id, linkRef, repoRoot ?? null),
      )
      return expectVariant(response, "WorkspaceLinkAttached")
    },
    detachWorkspaceLink: async (linkRef, repoRoot) => {
      const response = await client.send<Record<string, unknown>>(
        detachWorkspaceLinkRequest(sessionState().id, linkRef, repoRoot ?? null),
      )
      return expectVariant(response, "WorkspaceLinkDetached")
    },
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
          current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
        })
      }, 0)
    },
    cycleAgentFocus: async () => {
      return trackAgentFocusTransition(async () => {
        const response = await client.send<Record<string, unknown>>(
          cycleAgentFocusRequest(sessionState().id),
        )
        const payload = expectVariant<{ agent: AgentInstance | null }>(response, "AgentFocusCycled")
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent: payload.agent,
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
      const response = await client.send<Record<string, unknown>>(
        spawnAgentRequest(
          sessionState().id,
          provider,
          alias,
          model,
          worktreeId,
          effort,
          undefined,
          undefined,
          machineRef,
          worktreePlacement,
          sliceRef,
        ),
      )
      const payload = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned")
      return {
        agent: payload.agent,
        session: await getSessionState(client, sessionState().id),
      }
    },
    destroyAgent: async (agentId) => {
      await client.send<Record<string, unknown>>(
        destroyAgentRequest(sessionState().id, agentId),
      )
      return getSessionState(client, sessionState().id)
    },
    focusAgent: async (agentId) => {
      return trackAgentFocusTransition(async () => {
        const response = await client.send<Record<string, unknown>>(
          focusAgentRequest(sessionState().id, agentId),
        )
        const payload = expectVariant<{ agent: AgentInstance }>(response, "AgentFocused")
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent: payload.agent,
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

  const recoverProviderRun = async (reason: string) => {
    if (!isAttached() || providerRecoveryInFlight) {
      return
    }
    providerRecoveryInFlight = true
    try {
      const run = await launchProviderRun(
        client,
        sessionState().id,
        options.provider ?? "opencode",
        options.accountProfile,
        currentModelId(),
        currentVariantId(),
        focusedAgentId(),
      )
      setProviderRunState(run)
      applySessionState(applyProviderRunProfileToSession(await getSessionState(client, sessionState().id), run))
      await maybeResize(client, sessionState().id)
      setStatusLine("Recovered provider connection.")
      updateSessionChrome()
      flashFooter(`recovered provider run after ${reason}`, "info")
    } catch (error) {
      appLogger?.warn("provider recovery failed", {
        reason,
        error: formatError(error),
      })
    } finally {
      providerRecoveryInFlight = false
    }
  }

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

  const requestExit = async () => {
    if (closing && exitCleanupFailed) {
      appLogger?.warn("forcing cli exit after prior cleanup failure")
      await restoreTerminalAndExit(1)
      return
    }
    if (closing) {
      return
    }
    closing = true
    appLogger?.info("requested cli exit", {
      created_session: createdSessionState(),
    })
    try {
      syncPromptTextSnapshot()
      await flushPendingPromptDraftPersist().catch((error) => {
        appLogger?.warn("failed to flush prompt draft during exit", {
          error: formatError(error),
        })
      })
      const sessionId = attachmentState()?.session_id
      if (sessionId) {
        await persistSessionPromptState(sessionId, {
          promptDraft: persistablePromptDraft(),
        }).catch((error) => {
          appLogger?.warn("failed to persist prompt draft during exit", {
            session_id: sessionId,
            error: formatError(error),
          })
        })
      }
      const attachment = attachmentState()
      if (!attachment) {
        exitCleanupFailed = false
      } else if (shouldEndSessionOnCliExit(createdSessionState(), connectedClientCount())) {
        await client.send(endSessionRequest(sessionState().id))
      } else {
        await client.send(detachFromSessionRequest(attachment.id))
      }
      exitCleanupFailed = false
    } catch (error) {
      const decision = getExitCleanupDecision(error, exitCleanupFailed)
      exitCleanupFailed = true
      closing = false
      appLogger?.warn("exit cleanup failed", {
        error: formatError(error),
        will_exit: decision.exit,
      })
      appendNotice(decision.message, "warning")
      setStatusLine(decision.message)
      if (decision.exit) {
        await restoreTerminalAndExit(decision.exitCode)
      }
      return
    }
    appLogger?.info("cli exit cleanup completed")
    await restoreTerminalAndExit(0)
  }

  const requestWaitingRoom = async () => {
    if (closing) {
      return
    }
    appLogger?.info("requested waiting room", {
      created_session: createdSessionState(),
    })
    try {
      syncPromptTextSnapshot()
      await flushPendingPromptDraftPersist().catch((error) => {
        appLogger?.warn("failed to flush prompt draft during waiting-room transition", {
          error: formatError(error),
        })
      })
      const sessionId = attachmentState()?.session_id
      if (sessionId) {
        await persistSessionPromptState(sessionId, {
          promptDraft: persistablePromptDraft(),
        }).catch((error) => {
          appLogger?.warn("failed to persist prompt draft during waiting-room transition", {
            session_id: sessionId,
            error: formatError(error),
          })
        })
      }
      const attachment = attachmentState()
      if (attachment) {
        if (shouldEndSessionOnCliExit(createdSessionState(), connectedClientCount())) {
          await client.send(endSessionRequest(sessionState().id))
        } else {
          await client.send(detachFromSessionRequest(attachment.id))
        }
      }
    } catch (error) {
      appLogger?.warn("waiting room cleanup failed", {
        error: formatError(error),
      })
      appendNotice(formatError(error), "warning")
    }
    transitionToNoSession("Returned to waiting room.")
    appLogger?.info("waiting room transition completed")
  }

  const restoreTerminalAndExit = async (exitCode: number) => {
    try {
      renderer.disableKittyKeyboard()
    } catch {}
    try {
      renderer.disableStdoutInterception()
    } catch {}
    try {
      if (!renderer.isDestroyed) {
        renderer.destroy()
      }
    } catch (error) {
      appLogger?.warn("renderer destroy failed during exit", {
        error: formatError(error),
      })
    }
    await sleep(25)
    process.exit(exitCode)
  }

  const submitWorkspaceShellCommand = async (rawPrompt: string) => {
    const command = workspaceShellCommandText(rawPrompt)
    if (!command) {
      flashFooter("usage: @ <arroba-shell command>", "error")
      return { ok: false, output: "usage: @ <arroba-shell command>", context: workspaceShellContext() }
    }
    const context = workspaceShellContext()
    const output: string[] = []
    const result = await executeShellLine(command, context, { client }, (text) => output.push(text))
    const rendered = output.join("").trimEnd()
    const nextContext = result.context
    setWorkspaceShellContext(nextContext)
    setWorkspaceShellEntries((entries) => appendWorkspaceShellEntry(entries, {
      id: workspaceShellEntryCounter() + 1,
      command,
      output: rendered,
      ok: result.ok,
    }))
    setWorkspaceShellEntryCounter((counter) => counter + 1)

    const nextSessionId = nextContext.sessionId
    if (nextSessionId && nextSessionId === sessionState().id) {
      try {
        applySessionState(await getSessionState(client, nextSessionId))
      } catch (error) {
        appLogger?.warn("workspace shell session refresh failed", {
          session_id: nextSessionId,
          error: formatError(error),
        })
      }
    }

    const nextWorkflowId = nextContext.workflowId ?? null
    if (result.ok && nextWorkflowId) {
      const workflowExists = (sessionState().workflows ?? []).some((workflow) => workflow.id === nextWorkflowId)
      if (workflowExists && selectedWorkflowId() !== nextWorkflowId) {
        setSelectedWorkflowId(nextWorkflowId)
        setSelectedWorkflowNodeId(null)
      }
    }

    rebuildTranscript()
    flashFooter(result.ok ? "shell command completed" : (rendered || "shell command failed"), result.ok ? "info" : "error")
    return { ok: result.ok, output: rendered, context: nextContext }
  }

  const submitPrompt = async () => {
    if (!promptInput) {
      return
    }

    ensureBackgroundPollersStarted()

    const rawPrompt = promptInput.plainText
    const trimmed = rawPrompt.trim()
    if (!trimmed && pendingAttachments().length === 0) {
      promptInput.clear()
      syncPromptTextSnapshot()
      return
    }
    if (workflowScreenShowing() && isWorkspaceShellCommand(rawPrompt)) {
      try {
        await submitWorkspaceShellCommand(rawPrompt)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
      }
      return
    }
    if (workflowNodeInstructionsEditor() && !trimmed.startsWith("/")) {
      flashFooter("instructions editor is open; type in the I/O panel and use /workflow node instructions save", "info")
      promptInput.clear()
      syncPromptTextSnapshot()
      return
    }
    const allowSlashCommandSubmission = !workflowScreenShowing() || isWorkflowCommandInput(rawPrompt)
    const providerNamespaceCommand = parseProviderNamespaceCommand(
      rawPrompt,
      focusedBackendProvider(),
    )
    const slashCommand = allowSlashCommandSubmission ? parseSlashCommand(rawPrompt) : null
    if (slashCommand && isAttached()) {
      recordPromptAreaHistoryEntry(sessionState().id, rawPrompt)
    }
    const handledCommand = allowSlashCommandSubmission
      ? await executeSlashCommand(rawPrompt, {
      onExit: requestExit,
      onWaiting: requestWaitingRoom,
      onStop: requestPromptStop,
      onAttachment: async (command) => {
        try {
          await handleAttachmentCommand(command.raw)
        } catch (error) {
          appLogger?.error("attachment command failed", {
            command: trimmed,
            error: formatError(error),
          })
          flashFooter(formatError(error), "error")
        }
      },
      onSession: async (command) => {
        try {
          const handled = await handleSessionCommand(command)
          if (!handled) {
            flashFooter("unknown /session command", "error")
          }
        } catch (error) {
          appLogger?.error("session command failed", {
            command: trimmed,
            error: formatError(error),
          })
          flashFooter(formatError(error), "error")
        }
      },
      onProvider: async (command) => {
        try {
          await handleProviderCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onModel: async (command) => {
        try {
          await handleModelCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onVariant: async (command) => {
        try {
          await handleVariantCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
      onView: async (command) => {
        try {
          await handleViewCommand(command)
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
      },
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
      : null
    if (handledCommand) {
      promptInput.clear()
      syncPromptTextSnapshot()
      setPromptHistoryIndex(null)
      setPromptHistoryDraft(null)
      if (shouldClearCommandCenterForSlashCommand(handledCommand)) {
        clearCommandCenter()
      }
      return
    }
    if (providerNamespaceCommand) {
      const focusedProvider = focusedBackendProvider()
      if (providerNamespaceCommand.provider !== focusedProvider) {
        flashFooter(
          focusedProvider
            ? `${providerNamespaceCommand.raw.split(/\s+/, 1)[0]} is unavailable while the focused agent uses ${focusedProvider}`
            : "provider-native commands require a focused OpenCode, Codex, or Claude Code agent",
          "error",
        )
        return
      }
      if (!providerNamespaceCommand.forwardedCommand) {
        flashFooter(`usage: ${providerNamespaceCommand.raw} <provider-command>`, "error")
        return
      }
      if (workflowScreenShowing()) {
        flashFooter("provider-native commands are unavailable while the workflow screen owns the prompt", "error")
        return
      }
      if (pendingAttachments().length > 0) {
        flashFooter("provider-native commands do not support attachments", "error")
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
          promptInput.clear()
          syncPromptTextSnapshot()
          return
        }
        submissionUi = beginSubmittedPromptUi(rawPrompt)
        appendUserPrompt(renderPromptTranscript(providerNamespaceCommand.raw), targetAgentId)
        const forwardedPrompt = `${providerNamespaceCommand.forwardedCommand}\n`
        const response = await submitPromptWithRecovery(
          client,
          sessionState().id,
          attachment.id,
          targetAgentId,
          forwardedPrompt,
          [],
          options,
          appLogger,
        )
        const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
        payload.session = normalizeRuntimeSession(payload.session)
        const submittedTargetAgentId = submittedPromptTargetAgentId(payload) ?? targetAgentId
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
      promptInput.clear()
      syncPromptTextSnapshot()
      return
    }

    if (workflowScreenShowing()) {
      const workflowPrompt = workflowPromptState()
      if (!workflowPrompt.enabled) {
        flashFooter(`prompt disabled: ${workflowPrompt.disabledReason ?? "workflow prompt unavailable"}`, "info")
        return
      }
      if (pendingAttachments().length > 0) {
        flashFooter("workflow endpoint prompts do not support attachments", "error")
        return
      }
      if (!workflowPrompt.workflow || !workflowPrompt.endpoint) {
        flashFooter("workflow prompt target unavailable", "error")
        return
      }
      const workflowInvocationPrompt = rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`

      let submissionUi: SubmittedPromptUiSnapshot | null = null
      try {
        submissionUi = beginSubmittedPromptUi(rawPrompt)
        const payload = await invokeWorkflowEndpoint(
          workflowPrompt.workflow.id,
          workflowPrompt.endpoint.id,
          workflowInvocationPrompt,
        )
        if ("workflow_run" in payload) {
          flashFooter(
            `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
            "info",
          )
        } else {
          flashFooter(`queued workflow launch ${payload.queued_launch.id}`, "info")
        }
        recordPromptAreaHistoryEntry(sessionState().id, rawPrompt)
        return
      } catch (error) {
        restoreFailedPromptUi(submissionUi)
        flashFooter(formatError(error), "error")
        return
      }
    }

    const prompt = trimmed ? (rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`) : ""
    const rawAttachments = pendingAttachments().map<PromptAttachmentPart>((file) => ({
      url: file.url,
      mime: file.mime,
      filename: file.filename,
    }))
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
        promptInput.clear()
        syncPromptTextSnapshot()
        return
      }
      const attachments = await preparePromptAttachmentsForSubmit(rawAttachments, {
        inlineLocalFiles: Boolean(options.relayUrl) || promptAttachmentTransferIsForced(),
      })
      submissionUi = beginSubmittedPromptUi(rawPrompt)
      appendUserPrompt(renderPromptTranscript(prompt), targetAgentId)
      const response = await submitPromptWithRecovery(
        client,
        sessionState().id,
        attachment.id,
        targetAgentId,
        prompt,
        attachments,
        options,
        appLogger,
      )
      const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
      payload.session = normalizeRuntimeSession(payload.session)
      const submittedTargetAgentId = submittedPromptTargetAgentId(payload) ?? targetAgentId
      applySessionState(payload.session)
      setStreamingAgentId(submittedTargetAgentId)
      setWorking(true)
      updateSessionChrome()
      const outcomeName = firstVariantName(payload.outcome)
      appLogger?.info("prompt submitted", {
        outcome: outcomeName,
        active_prompt_id: payload.session.active_prompt?.id ?? null,
        queued_prompts: payload.session.queued_prompts.length,
      })
      setStatusLine(
        outcomeName === "Queued"
          ? `Prompt queued behind ${payload.session.active_prompt?.id ?? "the active turn"}.`
          : "Prompt submitted.",
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
    const attachment = attachmentState()
    if (stopRequestInFlight || !activePrompt() || !attachment) {
      return
    }

    stopRequestInFlight = true
    try {
      await client.send(cancelActivePromptRequest(sessionState().id, attachment.id))
      appLogger?.info("requested active prompt cancellation")
      setStatusLine("Cancellation requested.")
      setStreamingAgentId(activePrompt()?.target_agent_id ?? streamingAgentId())
      setWorking(true)
      updateSessionChrome()
    } catch (error) {
      stopRequestInFlight = false
      appLogger?.error("active prompt cancellation failed", {
        error: formatError(error),
      })
      setFatalError(formatError(error))
      updateSessionChrome()
    }
  }

  const submitFocusedInteractionChoice = async (choiceIndex?: number) => {
    const interaction = focusedAgentInteraction()
    if (!interaction || !isAttached()) {
      return false
    }
    const resolvedChoiceIndex = Math.min(
      choiceIndex ?? interactionChoiceSelection.get(interaction.id) ?? 0,
      Math.max(0, interaction.choices.length + (interaction.custom_choice ? 1 : 0) - 1),
    )
    const customChoice = interaction.custom_choice && resolvedChoiceIndex === interaction.choices.length
      ? interaction.custom_choice
      : null
    const choice = customChoice ? null : interaction.choices[resolvedChoiceIndex]
    if (!choice) {
      if (!customChoice) {
        return false
      }
      const reply = interactionCustomReplies.get(interaction.id) ?? ""
      const minLength = customChoice.min_length ?? 1
      if (reply.length < minLength) {
        interactionCustomEditing.add(interaction.id)
        renderAgentInteractions()
        applyResponseLayout()
        return true
      }
    }
    interactionChoiceSelection.set(interaction.id, resolvedChoiceIndex)
    try {
      const response = await client.send<Record<string, unknown>>(
        respondToInteractionRequest(
          sessionState().id,
          interaction.id,
          customChoice?.id ?? choice!.id,
          customChoice ? interactionCustomReplies.get(interaction.id) ?? "" : null,
        ),
      )
      const payload = expectVariant<{ session: RuntimeSession }>(response, "InteractionResponded")
      payload.session = normalizeRuntimeSession(payload.session)
      applySessionState(payload.session)
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
    const choiceCount = interaction.choices.length + (interaction.custom_choice ? 1 : 0)
    const nextIndex = (currentIndex + delta + choiceCount) % choiceCount
    interactionChoiceSelection.set(interaction.id, nextIndex)
    if (interaction.custom_choice && nextIndex !== interaction.choices.length) {
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
    const customIndex = interaction.custom_choice ? interaction.choices.length : -1
    const selectedIndex = interactionChoiceSelection.get(interaction.id) ?? 0
    if (interaction.custom_choice && interactionCustomEditing.has(interaction.id)) {
      if (event.name === "escape") {
        event.preventDefault?.()
        event.stopPropagation?.()
        interactionCustomEditing.delete(interaction.id)
        renderAgentInteractions()
        applyResponseLayout()
        return true
      }
      if (event.name === "backspace") {
        event.preventDefault?.()
        event.stopPropagation?.()
        const current = interactionCustomReplies.get(interaction.id) ?? ""
        interactionCustomReplies.set(interaction.id, current.slice(0, -1))
        renderAgentInteractions()
        applyResponseLayout()
        return true
      }
      if (event.name === "return" || event.name === "enter") {
        event.preventDefault?.()
        event.stopPropagation?.()
        void submitFocusedInteractionChoice(customIndex)
        return true
      }
      if (!event.ctrl && !event.meta && !event.alt && event.name.length === 1) {
        event.preventDefault?.()
        event.stopPropagation?.()
        const current = interactionCustomReplies.get(interaction.id) ?? ""
        const maxLength = interaction.custom_choice.max_length ?? 2000
        if (current.length < maxLength) {
          interactionCustomReplies.set(interaction.id, `${current}${event.name}`)
        }
        renderAgentInteractions()
        applyResponseLayout()
        return true
      }
      return true
    }
    if (event.name === "left" || event.name === "up") {
      event.preventDefault?.()
      event.stopPropagation?.()
      return cycleFocusedInteractionChoice(-1)
    }
    if (event.name === "right" || event.name === "down") {
      event.preventDefault?.()
      event.stopPropagation?.()
      return cycleFocusedInteractionChoice(1)
    }
    const numericIndex = Number.parseInt(event.name, 10)
    const choiceCount = interaction.choices.length + (interaction.custom_choice ? 1 : 0)
    if (Number.isInteger(numericIndex) && numericIndex >= 1 && numericIndex <= choiceCount) {
      event.preventDefault?.()
      event.stopPropagation?.()
      if (interaction.custom_choice && numericIndex - 1 === customIndex) {
        interactionChoiceSelection.set(interaction.id, customIndex)
        interactionCustomEditing.add(interaction.id)
        renderAgentInteractions()
        applyResponseLayout()
      } else {
        void submitFocusedInteractionChoice(numericIndex - 1)
      }
      return true
    }
    if (event.name === "return" || event.name === "enter") {
      event.preventDefault?.()
      event.stopPropagation?.()
      if (interaction.custom_choice && selectedIndex === customIndex && !(interactionCustomReplies.get(interaction.id) ?? "")) {
        interactionCustomEditing.add(interaction.id)
        renderAgentInteractions()
        applyResponseLayout()
      } else {
        void submitFocusedInteractionChoice()
      }
      return true
    }
    return false
  }

  useKeyboard((event) => {
    if (handleHotkeysToggleShortcut("keyboard", event)) {
      return
    }
    if (hotkeysOpen() && event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      closeHotkeys()
      return
    }
    if (sessionBrowserOpen() && event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      closeSessionBrowserDialog()
      return
    }
    if (terminalPairingOpen() && event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      closeTerminalPairingDialog()
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
    if (hotkeysOpen() || terminalPairingOpen() || sessionBrowserOpen()) {
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
    const direction = promptHistoryDirectionForKey({
      attached: isAttached(),
      promptFocused: Boolean(promptInput?.focused),
      commandCenterOpen: commandCenterOpen(),
      keyName: event.name,
      currentText: promptInput?.plainText ?? promptTextSnapshot,
      cursorOffset: promptInput?.cursorOffset ?? promptTextSnapshot.length,
      eventType: event.eventType,
      ctrl: event.ctrl,
      meta: event.meta,
      alt: event.alt,
      shift: event.shift,
    })
    if (!direction) {
      return false
    }
    if (direction === "next" && promptHistoryIndex() === null && promptHistoryDraft() === null) {
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
    if (!isAttached() || event.eventType === "release") {
      return false
    }
    if (event.name !== "up" && event.name !== "down") {
      return false
    }
    return Boolean(event.shift) && !(promptInput?.plainText.trim())
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
    if (event.eventType !== "release" && (hotkeysOpen() || terminalPairingOpen() || sessionBrowserOpen()) && event.name === "escape") {
      if (sessionBrowserOpen()) {
        closeSessionBrowserDialog()
      } else if (terminalPairingOpen()) {
        closeTerminalPairingDialog()
      } else {
        closeHotkeys()
      }
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
      if (hotkeysOpen() || terminalPairingOpen() || sessionBrowserOpen()) {
        return
      }
      toggleWorkspaceScreen()
      return
    }
    if (shouldCycleFocusOnTabEvent(event, {
      attached: isAttached(),
      hotkeysOpen: hotkeysOpen() || terminalPairingOpen() || sessionBrowserOpen(),
      promptFocused: Boolean(promptInput?.focused),
      commandCenterOpen: commandCenterOpen(),
      commandCenterQuery: commandCenterQuery(),
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
    if (hotkeysOpen() || terminalPairingOpen() || sessionBrowserOpen()) {
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
    if (event.eventType !== "release" && event.name === "backspace" && isAttached() && !promptInput?.plainText && pendingAttachments().length > 0) {
      removeLastPendingPromptAttachment()
      return
    }
    if (shouldNavigatePromptTurns(event)) {
      navigatePromptTurns(event.name === "up" ? "previous" : "next")
      return
    }
    if (shouldHandleWaitingRoomKeyEvent(event, {
      attached: isAttached(),
      hotkeysOpen: hotkeysOpen() || terminalPairingOpen() || sessionBrowserOpen(),
      promptFocused: Boolean(promptInput?.focused),
      commandCenterOpen: commandCenterOpen(),
      commandCenterQuery: commandCenterQuery(),
    })) {
      const keyName = event.name === "up" || event.name === "down" || event.name === "left" || event.name === "right"
        ? event.name
        : null
      if (keyName) {
        const next = {
          ...waitingRoomState(),
          keyState: {
            ...waitingRoomState().keyState,
            [keyName]: event.eventType !== "release",
          },
        }
        if (event.eventType !== "release") {
          reconcileWaitingRoom(
            keyName === "up"
              ? moveWaitingRoomFocus(next, availableSessions(), -1, {
                  relay: relayStatusState(),
                  machines: remoteMachinesState(),
                  kernels: remoteKernelsState(),
                  terminals: terminalsState(),
                  slices: slicesState(),
                })
              : keyName === "down"
                ? moveWaitingRoomFocus(next, availableSessions(), 1, {
                    relay: relayStatusState(),
                    machines: remoteMachinesState(),
                    kernels: remoteKernelsState(),
                    terminals: terminalsState(),
                    slices: slicesState(),
                  })
                : cycleWaitingRoomValue(next, availableSessions(), providerCatalogState(), keyName === "left" ? -1 : 1, themeRegistryState(), {
                    slices: slicesState(),
                  }),
          )
          return
        }
        setWaitingRoomState(next)
        rebuildTranscript()
        return
      }
      const sessionLifecycleAction = waitingRoomSessionLifecycleActionForEvent(event)
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
    const selectedWorkflow = sessionState().workflows?.find((workflow) => workflow.id === selectedWorkflowId()) ?? null
    return {
      screen: workspaceScreenMode(),
      workflowScreenActive: workflowScreenActive(),
      daemonDisconnected: daemonDisconnected(),
      statusLine: statusLine(),
      session: {
        id: sessionState().id,
        workspace: sessionState().workspace_id,
        worktree: sessionState().worktree_id,
        focusedAgentId: focusedAgentId(),
        agentCount: sessionState().agents.length,
        agents: sessionState().agents.map((agent) => {
          const badge = agentPaneStatusBadge(
            agent,
            agentActivityLabels()[agent.id] ?? null,
            hasPromptWorkByAgent()[agent.id] ?? false,
            agent.id === streamingAgentId(),
            agentBusyLatch(agent.id),
          )
          return {
            id: agent.id,
            alias: agent.alias,
            provider: agent.provider,
            state: agent.state,
            isProcessing: agent.is_processing,
            badge,
          }
        }),
      },
      interactions: (sessionState().active_interactions ?? []).map((interaction) => ({
        id: interaction.id,
        agentId: interaction.agent_id,
        kind: interaction.kind,
        level: interaction.level,
        title: interaction.title,
        message: interaction.message,
        timeoutSec: interaction.timeout_sec,
        defaultOnTimeout: interaction.default_on_timeout,
        focused: focusedAgentId() === interaction.agent_id,
        selectedChoiceIndex: interactionChoiceSelection.get(interaction.id) ?? 0,
        customChoice: interaction.custom_choice ?? null,
        customReply: interactionCustomReplies.get(interaction.id) ?? "",
        customEditing: interactionCustomEditing.has(interaction.id),
        choices: interaction.choices.map((choice) => ({
          id: choice.id,
          label: choice.label,
          style: choice.style,
        })),
      })),
      waitingRoom: !isAttached()
        ? {
          state: waitingRoomState(),
          rows: waitingRoomRows(waitingRoomState(), availableSessions(), providerCatalogState(), {
            cloudNotice: waitingRoomCloudNotice(),
            inventoryStatus: waitingRoomInventoryStatus(),
            loadingFrame: waitingRoomState().introStep,
            relay: relayStatusState(),
            machines: remoteMachinesState(),
            kernels: remoteKernelsState(),
            terminals: terminalsState(),
            slices: slicesState(),
          }, waitingRoomTargets(), themeRegistryState()).map((row) => ({
            id: row.id,
            title: row.title,
            value: row.value,
            focused: row.focused,
            selectable: row.selectable,
          })),
        }
        : null,
      selectedWorkflowId: selectedWorkflowId(),
      selectedWorkflowNodeId: selectedWorkflowNodeId(),
      selectedWorkflow: selectedWorkflow
        ? {
          id: selectedWorkflow.id,
          alias: selectedWorkflow.alias,
          nodeCount: selectedWorkflow.nodes?.length ?? 0,
          edgeCount: selectedWorkflow.edges?.length ?? 0,
          endpointCount: selectedWorkflow.endpoints?.length ?? 0,
        }
        : null,
      workflows: (sessionState().workflows ?? []).map((workflow) => ({
        id: workflow.id,
        alias: workflow.alias,
        nodeCount: workflow.nodes?.length ?? 0,
        edgeCount: workflow.edges?.length ?? 0,
        endpointCount: workflow.endpoints?.length ?? 0,
      })),
      workflowRuns: (sessionState().workflow_runs ?? []).map((run) => ({
        id: run.id,
        workflowId: run.workflow_id,
        endpointId: run.endpoint_id,
        status: run.status,
        nodeRunCount: run.node_runs?.length ?? 0,
        failureCount: run.failure_events?.length ?? 0,
        finalOutput: run.final_output ?? null,
      })),
      shell: {
        context: workspaceShellContext(),
        entries: workspaceShellEntries(),
        transcript: renderWorkspaceShellTranscript(workspaceShellEntries()),
      },
      footer: footerFlash(),
    }
  }

  const automationSnapshotMatches = (
    snapshot: ReturnType<typeof automationSnapshot>,
    request: CliAutomationRequest,
  ) => {
    if (typeof request.screen === "string" && snapshot.screen !== request.screen) {
      return false
    }
    if (typeof request.daemonDisconnected === "boolean" && snapshot.daemonDisconnected !== request.daemonDisconnected) {
      return false
    }
    if (typeof request.sessionId === "string" && snapshot.session.id !== request.sessionId) {
      return false
    }
    if (typeof request.statusLine === "string" && snapshot.statusLine !== request.statusLine) {
      return false
    }
    if (typeof request.selectedWorkflowAlias === "string" && snapshot.selectedWorkflow?.alias !== request.selectedWorkflowAlias) {
      return false
    }
    if (typeof request.workflowAlias === "string" && !snapshot.workflows.some((workflow) => workflow.alias === request.workflowAlias)) {
      return false
    }
    if (typeof request.shellEntryCount === "number" && snapshot.shell.entries.length < request.shellEntryCount) {
      return false
    }
    return true
  }

  const handleAutomationRequest = async (request: CliAutomationRequest): Promise<unknown> => {
    const action = typeof request.action === "string" ? request.action : ""
    switch (action) {
      case "ping":
        return { status: "ok" }
      case "switch_screen": {
        const screen = typeof request.screen === "string" ? request.screen : ""
        if (screen !== "agents" && screen !== "workflow") {
          throw new Error("usage: switch_screen screen=agents|workflow")
        }
        if (!isAttached()) {
          throw new Error("cannot switch screen without an attached session")
        }
        setWorkspaceScreenMode(screen)
        rebuildTranscript()
        applyResponseLayout()
        return automationSnapshot()
      }
      case "workspace_shell_exec": {
        const command = typeof request.command === "string" ? request.command : ""
        if (!command.trim()) {
          throw new Error("usage: workspace_shell_exec command=<arroba-shell command>")
        }
        if (!workflowScreenActive()) {
          showWorkflowScreen()
        }
        const result = await submitWorkspaceShellCommand(`@ ${command}`)
        return { result, snapshot: automationSnapshot() }
      }
      case "submit_prompt": {
        const prompt = typeof request.prompt === "string" ? request.prompt : ""
        if (!prompt.trim()) {
          throw new Error("usage: submit_prompt prompt=<text>")
        }
        const requestAttachments = Array.isArray(request.attachments)
          ? request.attachments.map((entry) => {
            if (!entry || typeof entry !== "object") {
              throw new Error("submit_prompt attachments must be objects")
            }
            const attachment = entry as Record<string, unknown>
            if (typeof attachment.url !== "string" || typeof attachment.mime !== "string") {
              throw new Error("submit_prompt attachments require url and mime")
            }
            return {
              url: attachment.url,
              mime: attachment.mime,
              filename: typeof attachment.filename === "string" ? attachment.filename : null,
            } satisfies PromptAttachmentPart
          })
          : []
        if (requestAttachments.length > 0) {
          if (!isAttached()) {
            throw new Error("cannot submit prompt attachments without an attached session")
          }
          const attachment = attachmentState()
          if (!attachment) {
            throw new Error("cannot submit prompt attachments without an attached client")
          }
          const attachments = await preparePromptAttachmentsForSubmit(requestAttachments, {
            inlineLocalFiles: Boolean(options.relayUrl) || promptAttachmentTransferIsForced(),
          })
          await submitPromptWithRecovery(
            client,
            sessionState().id,
            attachment.id,
            focusedAgentId(),
            prompt.endsWith("\n") ? prompt : `${prompt}\n`,
            attachments,
            options,
            appLogger,
          )
          return automationSnapshot()
        }
        if (isAttached()) {
          await launchProviderRun(
            client,
            sessionState().id,
            options.provider ?? "opencode",
            options.accountProfile,
            options.model,
            options.effort,
            focusedAgentId(),
          )
          await maybeResize(client, sessionState().id)
        }
        setPromptText(prompt)
        await submitPrompt()
        return automationSnapshot()
      }
      case "activate_waiting_room": {
        if (isAttached()) {
          throw new Error("cannot activate waiting room while attached")
        }
        await activateWaitingRoom()
        return automationSnapshot()
      }
      case "connect_detached_kernel": {
        if (kernelConnected()) {
          return automationSnapshot()
        }
        await connectDetachedKernelFromWaitingRoom()
        return automationSnapshot()
      }
      case "snapshot":
        return automationSnapshot()
      case "interaction_submit": {
        const choiceIndex = typeof request.choiceIndex === "number" ? request.choiceIndex : undefined
        await submitFocusedInteractionChoice(choiceIndex)
        return automationSnapshot()
      }
      case "interaction_move": {
        const delta = typeof request.delta === "number" ? request.delta : 0
        if (!delta) {
          throw new Error("usage: interaction_move delta=<signed integer>")
        }
        cycleFocusedInteractionChoice(delta)
        return automationSnapshot()
      }
      case "wait_for": {
        const timeoutMs = typeof request.timeoutMs === "number" ? request.timeoutMs : 10_000
        const intervalMs = typeof request.intervalMs === "number" ? request.intervalMs : 100
        const deadline = Date.now() + Math.max(1, timeoutMs)
        let snapshot = automationSnapshot()
        while (!automationSnapshotMatches(snapshot, request) && Date.now() < deadline) {
          await sleep(Math.max(10, intervalMs))
          snapshot = automationSnapshot()
        }
        if (!automationSnapshotMatches(snapshot, request)) {
          throw new Error("timed out waiting for CLI automation condition")
        }
        return snapshot
      }
      case "exit":
        void restoreTerminalAndExit(0)
        return { exiting: true }
      default:
        throw new Error(`unknown automation action '${action || String(request.action)}'`)
    }
  }

  const sendAutomationResponse = (socket: NetSocket, response: CliAutomationResponse) => {
    socket.write(`${JSON.stringify(response)}\n`)
  }

  const startAutomationServer = async (socketPath: string): Promise<NetServer> => {
    await unlink(socketPath).catch((error: NodeJS.ErrnoException) => {
      if (error.code !== "ENOENT") {
        throw error
      }
    })
    const server = createServer((socket) => {
      socket.setEncoding("utf8")
      let buffer = ""
      socket.on("data", (chunk) => {
        buffer += chunk
        while (buffer.includes("\n")) {
          const newlineIndex = buffer.indexOf("\n")
          const line = buffer.slice(0, newlineIndex).trim()
          buffer = buffer.slice(newlineIndex + 1)
          if (!line) {
            continue
          }
          let request: CliAutomationRequest
          try {
            request = JSON.parse(line) as CliAutomationRequest
          } catch (error) {
            sendAutomationResponse(socket, {
              id: null,
              ok: false,
              error: `invalid JSON automation request: ${formatError(error)}`,
            })
            continue
          }
          const id = typeof request.id === "string" || typeof request.id === "number" ? request.id : null
          void handleAutomationRequest(request)
            .then((data) => sendAutomationResponse(socket, { id, ok: true, data }))
            .catch((error) => sendAutomationResponse(socket, { id, ok: false, error: formatError(error) }))
        }
      })
    })
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject)
      server.listen(socketPath, () => {
        server.off("error", reject)
        resolve()
      })
    })
    appLogger?.info("cli automation socket listening", { socket_path: socketPath })
    return server
  }

  let automationServer: NetServer | null = null
  if (options.automationSocket) {
    void startAutomationServer(options.automationSocket)
      .then((server) => {
        automationServer = server
      })
      .catch((error) => {
        appLogger?.error("failed to start cli automation socket", {
          socket_path: options.automationSocket,
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
    if (automationServer) {
      automationServer.close()
      automationServer = null
    }
    if (options.automationSocket) {
      void unlink(options.automationSocket).catch(() => {})
    }
    if (pendingTerminalRecordFlush) {
      clearTimeout(pendingTerminalRecordFlush)
      pendingTerminalRecordFlush = undefined
    }
  })

  let pollersStarted = false
  const onResize = () => {
    if (isAttached()) {
      void maybeResize(client, sessionState().id)
    }
  }

  const monitorTranscriptScroll = () => {
    const scrollbox = transcriptScrollbox
    const decision = evaluateTranscriptScrollMonitor({
      hasScrollbox: Boolean(scrollbox),
      pendingHistoryScrollRestore,
      currentScrollTop: scrollbox?.scrollTop ?? 0,
      lastTranscriptScrollTop,
      hasMoreHistory: nextHistoryCursor() !== null,
      loadingHistory: loadingHistory(),
    })
    if (decision.shouldLoadOlderHistory) {
      void loadOlderHistoryPage()
    }
    lastTranscriptScrollTop = decision.nextLastScrollTop
  }

  const maybeLoadOlderHistoryForShortViewport = () => {
    const scrollbox = transcriptScrollbox
    if (shouldLoadShortViewportHistory({
      hasScrollbox: Boolean(scrollbox),
      attached: isAttached(),
      loadingHistory: loadingHistory(),
      hasMoreHistory: nextHistoryCursor() !== null,
      scrollTop: scrollbox?.scrollTop ?? 0,
      scrollHeight: scrollbox?.scrollHeight ?? 0,
      viewportHeight: scrollbox?.height ?? 0,
    })) {
      void loadOlderHistoryPage()
    }
  }

  const scheduleShortViewportHistoryCheck = () => {
    startTimeout(() => {
      maybeLoadOlderHistoryForShortViewport()
    }, 0)
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

  // Track daemon activity for connection health monitoring
  const recordDaemonActivity = (activityType: string) => {
    lastDaemonActivityAt = Date.now()
    consecutiveSilentPolls = 0
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

  const resyncAttachedKernelState = async (reason: string) => {
    if (kernelResyncInFlight) {
      return kernelResyncInFlight
    }
    kernelResyncInFlight = (async () => {
      const attachment = attachmentState()
      if (!attachment || !isAttached()) {
        return
      }
      const sessionId = sessionState().id
      appLogger?.info("resyncing attached kernel state", {
        reason,
        session_id: sessionId,
        attachment_id: attachment.id,
      })
      await catchUpAttachedSession(client, sessionId, attachment.id, sessionState(), appLogger)
      const previousSession = sessionState()
      const nextSession = await getSessionState(client, sessionId)
      if (!isAttached() || sessionState().id !== sessionId) {
        return
      }
      const projectedSession = applyProviderRunProfileToSession(nextSession, providerRunState())
      const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(projectedSession)
      const promptJustCompleted = sessionHasPromptWork(previousSession) && !sessionHasPromptWork(projectedSession)
      applySessionState(projectedSession)
      if (!nextSession.active_provider_run_id) {
        const activeRun = providerRunState()
        if (activeRun) {
          logProviderRunDebug("kernel resync cleared provider run", activeRun, {
            session_id: nextSession.id,
            reason,
          })
          setProviderRunState(null)
        }
      } else {
        const activeRun = providerRunState()
        const run = await tryGetProviderRun(client, nextSession.active_provider_run_id, appLogger)
        if (run && (!activeRun || !sameProviderRun(activeRun, run))) {
          logProviderRunDebug("kernel resync refreshed provider run", run, {
            session_id: nextSession.id,
            previous_provider_run_id: activeRun?.id ?? null,
            reason,
          })
          setProviderRunState(run)
          applySessionState(applyProviderRunProfileToSession(sessionState(), run))
        }
      }
      if (shouldRefreshPanes || promptJustCompleted || reason === "transport_resumed" || reason === "replay_gap") {
        await refreshAgentPanes(sessionState())
      }
      clearLocalBusyStateForAuthoritativeIdle(sessionState())
      recordDaemonActivity(`kernel_resync_${reason}`)
      setDaemonDisconnected(false)
      setStatusLine(DEFAULT_CONNECTED_STATUS)
      updateSessionChrome()
    })().catch((error) => {
      appLogger?.warn("attached kernel resync failed", {
        reason,
        error: formatError(error),
      })
      setDaemonDisconnected(true)
      setStatusLine("Waiting to reconnect to the Arroba kernel.")
      updateSessionChrome()
    }).finally(() => {
      kernelResyncInFlight = null
    })
    return kernelResyncInFlight
  }

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
        subscribedSessionId = null
        subscribedAttachmentId = null
        subscribedScope = null
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

  const recoverAttachedSessionAfterKernelRestart = () => {
    if (kernelRestartRecoveryInFlight) {
      return kernelRestartRecoveryInFlight
    }
    const sessionId = sessionState().id
    if (!isAttached() || !sessionId) {
      return null
    }
    kernelRestartRecoveryInFlight = (async () => {
      let delayMs = 250
      while (!closing && isAttached() && sessionState().id === sessionId && daemonDisconnected()) {
        try {
          const nextSession = await getSessionState(client, sessionId)
          if (!isAttached() || sessionState().id !== sessionId) {
            return
          }
          const nextAttachment = await attachToSession(client, sessionId, options.clientId)
          if (!isAttached() || sessionState().id !== sessionId) {
            return
          }
          setAttachmentState(nextAttachment)
          applySessionState(applyProviderRunProfileToSession(nextSession, providerRunState()))
          subscribedSessionId = null
          subscribedAttachmentId = null
          subscribedScope = null
          await syncKernelEventSubscription()
          await refreshAgentPanes(sessionState())
          clearLocalBusyStateForAuthoritativeIdle(sessionState())
          recordDaemonActivity("kernel_restart_recovered")
          setDaemonDisconnected(false)
          setStatusLine(DEFAULT_CONNECTED_STATUS)
          updateSessionChrome()
          appendNotice("Reconnected to the Arroba kernel.")
          return
        } catch (error) {
          appLogger?.debug("kernel restart recovery attempt failed", {
            session_id: sessionId,
            error: formatError(error),
          })
          await sleep(delayMs)
          delayMs = Math.min(delayMs * 2, 5_000)
        }
      }
    })().finally(() => {
      kernelRestartRecoveryInFlight = null
    })
    return kernelRestartRecoveryInFlight
  }

  async function syncKernelEventSubscription() {
    if (!supportsKernelEventStream) {
      return
    }

    const attachment = attachmentState()
    const sessionId = attachment ? sessionState().id : null
    appLogger?.debug("evaluating kernel event subscription", {
      session_id: sessionId,
      attachment_id: attachment?.id ?? null,
      subscribed_session_id: subscribedSessionId,
      subscribed_attachment_id: subscribedAttachmentId,
      subscribed_scope: subscribedScope,
      attached: Boolean(attachment),
    })

    if (!attachment || !sessionId) {
      if (subscribedScope === "waiting-room") {
        return
      }
      try {
        await client.subscribeToWaitingRoomInventory()
        subscribedScope = "waiting-room"
        subscribedAttachmentId = null
        subscribedSessionId = null
        appLogger?.info("subscribed to waiting room inventory events")
      } catch (error) {
        appLogger?.error("waiting room inventory subscription failed", {
          error: formatError(error),
        })
        setDaemonDisconnected(true)
        setStatusLine("Waiting to reconnect to the Arroba kernel.")
        appendNotice(`Waiting room inventory subscription failed: ${formatError(error)}`, "warning")
        updateSessionChrome()
      }
      return
    }

    if (subscribedScope === "session" && subscribedAttachmentId === attachment.id && subscribedSessionId === sessionId) {
      return
    }

    try {
      await client.subscribeToKernelEvents(sessionId, attachment.id)
      subscribedScope = "session"
      subscribedAttachmentId = attachment.id
      subscribedSessionId = sessionId
      appLogger?.info("subscribed to kernel events", {
        session_id: sessionId,
        attachment_id: attachment.id,
      })
    } catch (error) {
      appLogger?.error("kernel event subscription failed", {
        session_id: sessionId,
        attachment_id: attachment.id,
        error: formatError(error),
      })
      setDaemonDisconnected(true)
      setStatusLine("Waiting to reconnect to the Arroba kernel.")
      appendNotice(`Kernel event subscription failed: ${formatError(error)}`, "warning")
      updateSessionChrome()
    }
  }

  // Check if connection appears stale (working but no data received)
  const checkConnectionHealth = () => {
    const decision = evaluateConnectionHealth({
      attached: isAttached(),
      working: working(),
      now: Date.now(),
      lastDaemonActivityAt,
      consecutiveSilentPolls,
      silentThreshold: SILENT_POLL_THRESHOLD,
      silenceWindowMs: 2000,
    })
    consecutiveSilentPolls = decision.nextConsecutiveSilentPolls

    if (decision.shouldRecover) {
      appLogger?.warn("connection appears stale - no activity while working", {
        consecutive_silent_polls: consecutiveSilentPolls,
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
      consecutiveSilentPolls = 0
    }
  }

  const startConnectionWatchdog = () => {
    if (connectionWatchdogTimeout) {
      return
    }
    connectionWatchdogTimeout = startInterval(() => {
      if (closing) {
        clearInterval(connectionWatchdogTimeout)
        connectionWatchdogTimeout = undefined
        return
      }
      checkConnectionHealth()
    }, 250)
  }

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
      let response: Record<string, unknown>
      try {
        response = await client.send<Record<string, unknown>>(
          pumpTerminalOutputRequest(sessionState().id, attachment.id),
        )
      } catch (error) {
        const message = formatError(error)
        if (/has no active provider run/i.test(message) && !sessionHasPromptWork(sessionState())) {
          setProviderRunState(null)
          updateSessionChrome()
          return
        }
        throw error
      }
      const payload = expectVariant<{ records: TerminalOutputRecord[] }>(response, "TerminalOutput")
      if (payload.records.length > 0) {
        recordDaemonActivity("terminal_output")
      }
      queueTerminalOutputRecords(payload.records)
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
      const response = await client.send<Record<string, unknown>>(
        pollRuntimeNoticesRequest(sessionState().id, attachment.id),
      )
      recordDaemonActivity("runtime_notices")
      const payload = expectVariant<{ notices: RuntimeNoticeRecord[] }>(response, "RuntimeNotices")
      for (const notice of payload.notices) {
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
      const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionState().id))
      recordDaemonActivity("session_state_poll")
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
      payload.session = normalizeRuntimeSession(payload.session)
      const projectedSession = applyProviderRunProfileToSession(payload.session, providerRunState())
      const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(projectedSession)
      const promptJustCompleted = sessionHasPromptWork(previousSession) && !sessionHasPromptWork(projectedSession)
      applySessionState(projectedSession)
      if (shouldRefreshPanes || promptJustCompleted) {
        await refreshAgentPanes(projectedSession)
      }
      if (payload.session.active_provider_run_id) {
        const activeRun = providerRunState()
        const run = await tryGetProviderRun(client, payload.session.active_provider_run_id, appLogger)
        if (run && (!activeRun || !sameProviderRun(activeRun, run))) {
          logProviderRunDebug("session poll refreshed provider run", run, {
            session_id: payload.session.id,
            previous_provider_run_id: activeRun?.id ?? null,
            previous_model: activeRun?.model ?? null,
            previous_variant: activeRun?.variant ?? null,
            previous_usage_tokens_total: activeRun?.usage_tokens_total ?? null,
            refresh_reason: !activeRun
              ? "missing_run"
              : activeRun.id !== payload.session.active_provider_run_id
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
          session_id: payload.session.id,
        })
        setProviderRunState(null)
        updateSessionChrome()
        if (sessionHasPromptWork(payload.session)) {
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
    process.stdout.off("resize", onResize)
  })

  onCleanup(() => {
    if (footerFlashTimeout) {
      clearTimeout(footerFlashTimeout)
    }
    clearPendingPromptDraftPersist()
    if (pendingTurnCompletion) {
      clearTimeout(pendingTurnCompletion)
    }
    if (pendingSessionChromeFlush) {
      clearTimeout(pendingSessionChromeFlush)
    }
    if (pendingPromptInputHistoryRefresh) {
      clearTimeout(pendingPromptInputHistoryRefresh)
    }
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
    monitorTranscriptScroll()
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
        value.syntaxStyle = promptTokenStyle
        syncPromptPlaceholder()
        if (promptTextSnapshot) {
          setPromptText(promptTextSnapshot)
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

function reindexTranscriptEntries(entries: TranscriptEntry[], startingId: number): TranscriptEntry[] {
  return entries.map((entry, index) => ({
    ...entry,
    id: startingId + index + 1,
  }))
}

async function submitPromptWithRecovery(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
  options: CliOptions,
  logger?: ArrobaLogger | null,
): Promise<Record<string, unknown>> {
  try {
    return await client.send<Record<string, unknown>>(
      submitPromptRequest(sessionId, attachmentId, targetAgentId, prompt, attachments),
    )
  } catch (error) {
    if (!isRecoverableProviderError(error)) {
      throw error
    }

    logger?.warn("prompt submission hit recoverable provider error", {
      error: formatError(error),
      session_id: sessionId,
    })
    await launchProviderRun(
      client,
      sessionId,
      options.provider ?? "opencode",
      options.accountProfile,
      options.model,
      options.effort,
      targetAgentId,
    )
    await maybeResize(client, sessionId)
    logger?.info("relaunched provider after recoverable prompt failure", {
      session_id: sessionId,
    })
    return client.send<Record<string, unknown>>(
      submitPromptRequest(sessionId, attachmentId, targetAgentId, prompt, attachments),
    )
  }
}

function submittedPromptTargetAgentId(payload: PromptSubmittedPayload) {
  const outcome = payload.outcome as Record<string, unknown>
  const variant = Object.values(outcome)[0]
  if (!variant || typeof variant !== "object") {
    return null
  }
  const prompt = (variant as { prompt?: { target_agent_id?: unknown } }).prompt
  return typeof prompt?.target_agent_id === "string" ? prompt.target_agent_id : null
}

function isRecoverableProviderError(error: unknown): boolean {
  const message = formatError(error)
  return message.includes("has no active provider run") || message.includes("cannot perform `submit prompt` while ended")
}

function isSessionUnavailableError(error: unknown): boolean {
  const message = formatError(error)
  return /session `[^`]+` was not found/i.test(message)
    || /attachment `[^`]+` was not found/i.test(message)
    || /does not belong to session/i.test(message)
    || /cannot perform `[^`]+` while ended/i.test(message)
}

function isTerminalPairingLink(value: string) {
  return value.trim().startsWith("arroba-terminal-pair-v1.")
}

function applyTerminalPairingLinkOptions(options: CliOptions, pairingLink: string) {
  const parsed = parseTerminalPairingLink(pairingLink)
  options.relayUrl = parsed.relayUrl
  options.relayToken = parsed.relayToken
  options.targetDaemonId = parsed.targetDaemonId
  if (parsed.targetDaemonAlias) {
    options.targetDaemonAlias = parsed.targetDaemonAlias
  }
  options.clientId = parsed.terminalId ?? options.clientId
}

function parseTerminalPairingLink(pairingLink: string) {
  const payload = pairingLink.trim().replace(/^arroba-terminal-pair-v1[.]/, "")
  let decoded: Record<string, unknown>
  try {
    decoded = JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as Record<string, unknown>
  } catch (error) {
    throw new Error(`invalid terminal pairing link: ${formatError(error)}`)
  }
  const relayUrl = typeof decoded.relay_url === "string" ? decoded.relay_url : ""
  const relayToken = typeof decoded.relay_token === "string" ? decoded.relay_token : ""
  const targetDaemonId = typeof decoded.target_daemon_id === "string" ? decoded.target_daemon_id : ""
  if (!relayUrl || !relayToken || !targetDaemonId) {
    throw new Error("invalid terminal pairing link: missing relay target details")
  }
  return {
    relayUrl,
    relayToken,
    targetDaemonId,
    targetDaemonAlias: typeof decoded.target_daemon_alias === "string" ? decoded.target_daemon_alias : undefined,
    terminalId: typeof decoded.terminal_id === "string" ? decoded.terminal_id : undefined,
  }
}

function parseArgs(args: string[]): CliOptions {
  const options: CliOptions = {
    clientId: `arroba-cli-${process.pid}`,
    provider: "opencode",
    model: "default",
    accountProfile: "default",
    effort: "",
  }

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] ?? ""
    const next = () => {
      const value = args[index + 1]
      if (!value) {
        throw new Error(`missing value for ${arg}`)
      }
      index += 1
      return value
    }

    switch (arg) {
      case "--detached":
        options.detached = true
        break
      case "--socket":
        options.socketPath = next()
        break
      case "--automation-socket":
        options.automationSocket = path.resolve(next())
        break
      case "--kernel-url":
        options.kernelUrl = next()
        break
      case "--session":
        options.sessionId = next()
        break
      case "--relay-url":
        options.relayUrl = next()
        break
      case "--relay-token":
        options.relayToken = next()
        break
      case "--target-daemon-id":
        options.targetDaemonId = next()
        break
      case "--target-daemon-alias":
        options.targetDaemonAlias = next()
        break
      case "--terminal-pairing-link":
      case "--pairing-link":
        applyTerminalPairingLinkOptions(options, next())
        break
      case "--create-session":
        options.createSession = true
        break
      case "--delete-session":
        options.deleteSessionRef = next()
        break
      case "--alias":
        options.alias = next()
        break
      case "--client-id":
        options.clientId = next()
        break
      case "--model":
        options.model = next()
        break
      case "--provider":
        options.provider = next()
        break
      case "--account-profile":
        options.accountProfile = next()
        break
      case "--effort":
        options.effort = next()
        break
      case "--workspace":
        options.workspace = path.resolve(next())
        break
      case "--worktree":
        options.worktree = path.resolve(next())
        break
      case "--help":
      case "-h":
        printUsage()
        process.exit(0)
      default:
        if (isTerminalPairingLink(arg)) {
          applyTerminalPairingLinkOptions(options, arg)
          break
        }
        throw new Error(`unknown argument ${arg}`)
    }
  }

  if (options.createSession && options.sessionId) {
    throw new Error("--create-session cannot be used together with --session")
  }
  if (options.detached && (options.createSession || options.sessionId || options.deleteSessionRef)) {
    throw new Error("--detached cannot be used together with --create-session, --session, or --delete-session")
  }
  if (options.relayUrl && !options.relayToken) {
    throw new Error("--relay-url requires --relay-token")
  }
  if (options.relayUrl && !options.targetDaemonId && !options.targetDaemonAlias) {
    throw new Error("--relay-url requires --target-daemon-id or --target-daemon-alias")
  }
  if (options.relayUrl && (options.kernelUrl || options.socketPath)) {
    throw new Error("--relay-url cannot be used together with --kernel-url or --socket")
  }
  if (options.createSession && options.deleteSessionRef) {
    throw new Error("--create-session cannot be used together with --delete-session")
  }
  if (options.alias && !options.createSession) {
    throw new Error("--alias requires --create-session")
  }

  return options
}

async function getWaitingRoomInventory(client: LocalIpcClient): Promise<{
  inventoryVersion: string
  sessions: WaitingRoomPublicSessionSummary[]
  relayStatus: RelayStatusView
  remoteMachines: RemoteMachineView[]
  remoteKernels: RemoteKernelView[]
  terminals: TerminalView[]
  slices: SliceRecord[]
}> {
  const response = await client.send<Record<string, unknown>>(getWaitingRoomPublicSnapshotRequest())
  const payload = expectVariant<{
    snapshot: WaitingRoomPublicSnapshot
  }>(response, "WaitingRoomPublicSnapshot").snapshot
  const slices = await listSlices(client).catch(() => [])
  return {
    inventoryVersion: payload.inventory_version,
    sessions: payload.sessions.slice().sort((left, right) => right.created_at_ms - left.created_at_ms),
    relayStatus: payload.relay_status,
    remoteMachines: payload.remote_machines,
    remoteKernels: payload.remote_kernels,
    terminals: payload.terminals ?? [],
    slices,
  }
}

async function getSessionHistory(
  client: LocalIpcClient,
  sessionId: string,
  cursor?: SessionHistoryCursor | null,
  agentId?: string | null,
): Promise<SessionHistoryPage> {
  const response = await client.send<Record<string, unknown>>(
    getSessionHistoryRequest(sessionId, HISTORY_PAGE_ROUND_COUNT, BOOTSTRAP_HISTORY_MAX_CHARS, cursor, agentId),
  )
  return expectVariant<SessionHistoryPage>(response, "SessionHistory")
}

async function getPromptInputHistory(
  client: LocalIpcClient,
  sessionId: string,
  afterSequence: number | null = null,
  limit = 5000,
): Promise<PromptInputHistoryPage> {
  const response = await client.send<Record<string, unknown>>(
    getPromptInputHistoryRequest(sessionId, afterSequence, limit),
  )
  return expectVariant<PromptInputHistoryPage>(response, "PromptInputHistory")
}

function maxPromptInputHistorySequence(entries: readonly PromptInputHistoryPage["entries"][number][]) {
  return entries.reduce((max, entry) => Math.max(max, entry.sequence), 0)
}

function promptHistoryEntryListsEqual(left: readonly string[], right: readonly string[]) {
  return left.length === right.length && left.every((entry, index) => entry === right[index])
}

async function catchUpAttachedSession(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  session: RuntimeSession,
  logger?: ArrobaLogger | null,
): Promise<void> {
  if (!session.active_provider_run_id && !sessionHasPromptWork(session)) {
    return
  }

  try {
    await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
    await client.send<Record<string, unknown>>(pollRuntimeNoticesRequest(sessionId, attachmentId))
  } catch (error) {
    logger?.warn("attached session catch-up failed", {
      session_id: sessionId,
      attachment_id: attachmentId,
      error: formatError(error),
    })
  }
}

async function maybeResize(client: LocalIpcClient, sessionId: string): Promise<void> {
  if (!process.stdout.isTTY || !process.stdout.columns || !process.stdout.rows) {
    return
  }
  await client.send<Record<string, unknown>>(resizeTerminalRequest(sessionId, process.stdout.columns, process.stdout.rows))
}

function defaultKernelEndpoint(): string {
  if (process.env.ARROBA_KERNEL_URL) {
    return process.env.ARROBA_KERNEL_URL
  }
  const host = process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"
  const port = process.env.ARROBA_KERNEL_PORT ?? "43118"
  return `ws://${host}:${port}/kernel`
}

function resolveConfiguredCloudRelayApiUrl(preferences: ArrobaPreferences): string | undefined {
  const configured = process.env.ARROBA_CLOUD_API_URL
    ?? process.env.ARROBA_CLOUD_HOSTED_API_URL
    ?? preferences.relay?.cloud?.apiUrl
  return configured?.trim().replace(/\/+$/, "") || undefined
}

async function openExternalUrl(url: string): Promise<boolean> {
  const command = process.platform === "darwin"
    ? "open"
    : process.platform === "win32"
      ? "cmd"
      : "xdg-open"
  const args = process.platform === "win32" ? ["/c", "start", "", url] : [url]
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      detached: true,
      stdio: "ignore",
    })
    child.once("error", () => resolve(false))
    child.once("spawn", () => {
      child.unref()
      resolve(true)
    })
  })
}

function trimSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text
}

function formatError(error: unknown): string {
  return describeCliError(error)
}

function printUsage() {
  process.stdout.write(
    "usage: arroba-cli [--detached] [--kernel-url URL] [--socket PATH] [--automation-socket PATH] [--terminal-pairing-link LINK] [--relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)] [--session REF] [--create-session] [--alias NAME] [--delete-session REF] [--client-id ID] [--provider NAME] [--model MODEL] [--account-profile PROFILE] [--effort LEVEL] [--workspace PATH] [--worktree PATH]\n       arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]\n\ncommands:\n  /stop                 request cancellation of the active provider turn\n  /exit                 exit the CLI\n  /waiting              go to the waiting room\n  /provider <name>      select the provider backend\n  /provider status [n]  show auth status for the current or named provider\n  /provider login [n]   start provider-native login for the current or named provider\n  /provider logout [n]  clear the current or named provider login\n  /provider reauth [n]  log out then start a fresh provider login\n  /model <id>           select the active model\n  /variant <name>       select the model variant\n  /workspace [path]     show or set the next-session workspace path\n  /workspace link ...   manage workspace links for the attached session\n  /worktree [path]      show or set the next-session worktree path\n  /worktree create <branch> [directory] [--from <ref>] create a named git worktree\n  /worktree name [a]    set or clear the current worktree display name\n  /view <mode>          set multi-agent response layout to split|individual\n  /session new [d]      create and attach to a new session, optionally in directory d\n  /session create [d]   alias for /session new\n  /session <a>          alias the current session\n  /session attach <r>   attach to a session by id or alias\n  /session delete [r]   delete the current or referenced session\n  /agent spawn [a] [m] [--dir d] [--worktree d --branch b] [--machine r|--slice s] spawn a local, remote, or slice agent\n  /agent delete [r]     delete the focused or referenced agent\n  /agent destroy [r]    alias for /agent delete\n  /agent focus <id>     focus a specific agent\n  /agent list           list all agents in the session\n  /agent cycle          cycle to the next agent (or use Tab)\n  /machine list         list approved, pending, and offline remote machines\n  /machine kernels <m>  list live kernels for a remote machine\n  /machine approve <m>  approve a pending remote machine for spawning\n  /machine forget <m>   forget a registered remote machine\n  /machine rename <m> <alias> rename and approve a remote machine\n  /slice list           list slices owned by this kernel\n  /slice create <n>     create a slice inventory entry\n  /slice status [s]     show a slice, defaulting to focused agent slice\n  /slice screen [s]     open or print the slice screen URL\n  /config show          show the Arroba user config\n  /config keys          list settable config keys\n  /config schema        show config key metadata\n  /config set <p> <v>   update the Arroba user config\n  /config managed-io required|unrestricted set global managed I/O\n  /opencode <cmd>       forward an OpenCode-native command to the focused OpenCode agent\n  /codex <cmd>          forward a Codex-native command to the focused Codex agent\n  /workflow             open the workflow outline\n  /workflow list        list workflows in the workspace\n  /workflow show [r]    show selected workflow or workflow by id/alias\n  /workflow new [a]     create a new workflow with an optional alias\n  /workflow run [w] <e> [p] invoke a workflow endpoint with an optional prompt\n  /workflow runs [w]    list workflow runs for the session or one workflow\n  /workflow cancel <r>  cancel a workflow run\n  /workflow resume <r>  resume a stopped workflow run\n  /workflow terminal [w] show the workflow terminal in the I/O panel\n  /workflow watchdog ... manage scheduled endpoint triggers\n  /workflow <id> <a>    assign an alias to an existing workflow\n  /workflow <w> <f> <t> shorthand for /workflow edge add using node ids or agent refs\n  /workflow node ...    add/remove workflow nodes; /workflow add node all adds missing agents\n  /workflow edge ...    add/remove workflow edges; workflow id may be omitted\n  /workflow endpoint ... manage workflow endpoints; workflow id may be omitted\n  Tab                   keyboard shortcut to cycle focus\n  Ctrl+Tab              switch between the agent screens and workflow outline\n",
  )
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
