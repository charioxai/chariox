import path from "node:path"
import process from "node:process"
import { homedir } from "node:os"
import { pathToFileURL } from "node:url"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, DiffRenderable, MarkdownRenderable, MouseButton, RGBA, ScrollBoxRenderable, SyntaxStyle, TextAttributes, TextNodeRenderable, TextRenderable, addDefaultParsers, parseKeypress, type KeyBinding, type Renderable, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { batch, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js"
import { createStore, produce, reconcile } from "solid-js/store"

import type {
  AgentInstance,
  BootstrapState,
  CaptureScreenshotResult,
  CliOptions,
  PromptAttachmentPart,
  PromptSubmittedPayload,
  ReadDirectoryTreeResult,
  RuntimeAttachment,
  RuntimeNoticeRecord,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
  SessionHistoryCursor,
  SessionHistoryEntry,
  SessionHistoryPage,
  SessionHistoryPageEntry,
  StoredTransferArtifact,
  TerminalOutputRecord,
  TranscriptEntry,
} from "./cli-types.js"
import { buildCommandCenterItems, type CommandCenterItem } from "./command-center.js"
import { refreshAgentPaneState, trimAgentPaneEntries } from "./agent-pane-state.js"
import { copyTextToClipboard } from "./clipboard.js"
import { HOTKEY_TOGGLE_LABEL, matchHotkeysToggleEvent } from "./hotkeys.js"
import { computeCollapsedHistoryScrollTop, computePrependedHistoryScrollTop, findTurnPromptScrollTarget } from "./history-viewport.js"
import { LocalIpcClient } from "./ipc.js"
import {
  attachToSessionRequest,
  cancelActivePromptRequest,
  captureScreenshotRequest,
  createSessionRequest,
  cycleAgentFocusRequest,
  deleteSessionRequest,
  destroyAgentRequest,
  detachFromSessionRequest,
  endSessionRequest,
  focusAgentRequest,
  getProviderCatalogRequest,
  getProviderRunRequest,
  getSessionHistoryRequest,
  getSessionStateRequest,
  launchProviderRunRequest,
  listSessionsRequest,
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
  readDirectoryTreeRequest,
  resizeTerminalRequest,
  resolveSessionRequest,
  spawnAgentRequest,
  storeTransferredFileRequest,
  submitPromptRequest,
  updateSessionConfigRequest,
} from "./ipc-requests.js"
import { createProcessLogger, type ArrobaLogger } from "./logging.js"
import { runLogViewer } from "./logs.js"
import { loadPreferences, saveProviderPreferences, saveUiPreferences, type ArrobaPreferences, type MultiAgentResponseLayout } from "./preferences.js"
import {
  extractDroppedPromptAttachments,
  parsePromptAttachmentCommand,
  resolvePromptAttachmentEdit,
  type ParsedPromptAttachment,
  type PromptAttachmentKind,
} from "./prompt-attachments.js"
import type { PromptMetaPart, PromptMetaTone } from "./prompt-meta.js"
import {
  fallbackProviderCatalog,
  selectConfiguredModel,
  type ProviderCatalog,
} from "./provider-catalog.js"
import { computeSplitPaneGeometry, selectResponsePaneAgents, splitPaneAuxiliaryAgentIds } from "./response-panes.js"
import {
  STATUS_BADGE_WIDTH,
  DEFAULT_CONNECTED_STATUS,
  describeCliError,
  getExitCleanupDecision,
  getPollRecoveryDecision,
  getProviderActivityLabel,
  getSessionStatusLabel,
  getToolActivityLabel,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
import {
  deriveAttachedFooterSummary,
  deriveCurrentProviderSelection,
  deriveFooterHint,
  deriveFocusedStatusBadge,
  derivePromptMetaState,
  derivePromptUsageState,
  deriveSessionStatusMode,
  deriveVisibleActivityLabel,
  type SessionStatusMode,
} from "./session-chrome-state.js"
import {
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  buildDetachedSessionState,
  deriveSessionTransitionState,
  sessionHasPromptWork,
  sessionResponseLayout,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "./session-state.js"
import {
  formatToolTranscriptUpdate,
  guessPathFenceLanguage,
  mergeToolTranscriptUpdate,
  normalizeMarkdownFenceInfoStrings,
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  splitInlineCodeSpans,
  shouldRenderTranscriptAsMarkdown,
  shouldRenderProviderStatus,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import {
  decideBootstrapAction,
  SESSION_NEW_ERROR_HINT,
  SESSION_NEW_FOOTER_HINT,
  SESSION_NEW_HELP_TEXT,
  SESSION_NEW_PLACEHOLDER,
  formatSessionList,
  selectAttachableSession,
} from "./sessions.js"
import {
  buildSplitPaneFooterState,
  reflectedDistance,
  type StatusBadgeTone,
} from "./split-pane-footer.js"
import {
  applyResponseLayoutRenderables,
  requestRenderableTreeRender,
  syncAuxiliaryPane,
} from "./response-layout-render.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { createTranscriptSyntaxStyle, EmptyBorder, PromptBorderChars, SplitBorder, theme } from "./theme.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomModelSelectionDecision,
  deriveWaitingRoomStateUpdate,
  deriveWaitingRoomVariantSelectionDecision,
} from "./waiting-room-controller.js"
import {
  arrobaArtFrame,
  createWaitingRoomState,
  cycleWaitingRoomValue,
  moveWaitingRoomFocus,
  normalizeWaitingRoomState,
  waitingRoomRows,
  type WaitingRoomFocus,
  type WaitingRoomState,
} from "./waiting-room.js"
import {
  buildDirectoryTreeRows,
  createDirectoryTreeState,
  isDirectoryTreePathLoaded,
  mergeDirectoryTreeEntries,
  moveDirectoryTreeSelection,
  toggleDirectoryTreeExpansion,
  type DirectoryTreeEntry,
  type DirectoryTreeState,
} from "./tree-view.js"
import parserConfig from "./parsers-config.js"

const PROMPT_KEYBINDINGS = [
  { name: "return", action: "submit" },
  { name: "return", shift: true, action: "newline" },
  { name: "return", meta: true, action: "newline" },
] satisfies KeyBinding[]

const BOOTSTRAP_HISTORY_MAX_CHARS = 100_000
const HISTORY_PAGE_ROUND_COUNT = 1
const LIVE_TRANSCRIPT_LIMIT = 400
const LIVE_TRANSCRIPT_MAX_CHARS = 250_000
const STREAM_BATCH_WINDOW_MS = 16
const TURN_COMPLETION_SETTLE_MS = 150
const COMMAND_CENTER_OVERLAY_FOOTPRINT = 3
const ATTACHED_PROMPT_PLACEHOLDER = "Write your next prompt here"
const HOTKEY_DIALOG_WIDTH = 72

type HotkeyItem = {
  keys: string
  description: string
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
  { keys: "Tab", description: "Toggle between the transcript and file tree." },
  { keys: "Ctrl+A", description: "Cycle focus to the next agent." },
  { keys: "Up / Down", description: "Jump between user turns when the prompt is empty." },
  { keys: "Backspace / Delete", description: "Remove pending attachment tokens from the prompt." },
  { keys: "Tree: Up / Down / Enter", description: "Move and toggle the file tree when it is active." },
]

const WAITING_ROOM_HOTKEYS: HotkeyItem[] = [
  { keys: "Arrow keys", description: "Move between sessions, models, and effort levels." },
  { keys: "Enter", description: "Create or attach to the selected session." },
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

type TranscriptEntryRenderable = {
  entry: TranscriptEntry
  wrapper: BoxRenderable
  update: (entry: TranscriptEntry) => void
}

type TranscriptSurfaceTone = "default" | "focused" | "faded"

function shouldDeferHistoryEntry(entry: TranscriptEntry) {
  return entry.historyFragmentStart !== undefined && entry.historyFragmentStart > 0
}

function applyHistoryDeferral(entry: TranscriptEntry) {
  const deferred = shouldDeferHistoryEntry(entry)
  if (deferred) {
    entry.historyDeferred = true
  } else {
    delete entry.historyDeferred
  }
  return entry
}

function markDeferredHistoryEntries(items: TranscriptEntry[]) {
  if (items.length === 0) {
    return items
  }
  return items.map((entry, index) => {
    if (index === 0) {
      return applyHistoryDeferral({ ...entry })
    }
    if (!entry.historyDeferred) {
      return entry
    }
    const next = { ...entry }
    delete next.historyDeferred
    return next
  })
}

function collapseHistoricalTurns(entries: TranscriptEntry[], keepLatestExpanded = true) {
  const normalizedEntries = normalizeTranscriptTurnIds(entries)
  const latestTurnId = keepLatestExpanded ? computeCurrentTurnId(normalizedEntries) : null
  if (keepLatestExpanded && !latestTurnId) {
    return normalizedEntries
  }

  let nextId = normalizedEntries.reduce((max, entry) => Math.max(max, entry.id), 0)
  const result: TranscriptEntry[] = normalizedEntries
    .filter((entry) => entry.role !== "turn_summary" && entry.role !== "turn_toggle")
    .map((entry) => {
      const next: TranscriptEntry = { ...entry }
      if (latestTurnId !== null && entry.turnId === latestTurnId) {
        next.hidden = false
      }
      return next
    })
  const turnIds = [...new Set(result.map((entry) => entry.turnId).filter((turnId): turnId is number => typeof turnId === "number"))]

  for (const turnId of turnIds) {
    if (latestTurnId !== null && turnId === latestTurnId) {
      for (const entry of result) {
        if (entry.turnId === turnId) {
          entry.hidden = false
        }
      }
      continue
    }
    const promptIndex = result.findIndex((entry) => entry.turnId === turnId && entry.role === "user")
    const anchorIndex = promptIndex >= 0
      ? promptIndex
      : result.findIndex((entry) => entry.turnId === turnId)
    if (anchorIndex === -1) {
      continue
    }
    const turnEntries = result.filter((entry) => entry.turnId === turnId && entry.role !== "user")
    if (turnEntries.length === 0) {
      continue
    }
    const collapsedText = collapsedTurnText(turnEntries)
    for (const entry of turnEntries) {
      entry.hidden = true
    }
    const inserts: TranscriptEntry[] = []
    if (collapsedText) {
      inserts.push({
        id: ++nextId,
        role: "turn_summary",
        text: collapsedText,
        turnId,
      })
    }
    inserts.push({
      id: ++nextId,
      role: "turn_toggle",
      text: "click to expand turn",
      turnId,
      toggleMode: "expand",
    })
    result.splice(anchorIndex + 1, 0, ...inserts)
  }

  return result
}

function expandLatestTurnForLiveUpdateInPlace(entries: TranscriptEntry[]) {
  const latestTurnId = computeCurrentTurnId(entries)
  if (!latestTurnId) {
    return false
  }

  let changed = false
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index]
    if (!entry || entry.turnId !== latestTurnId) {
      continue
    }
    if (entry.role === "turn_toggle" || entry.role === "turn_summary") {
      entries.splice(index, 1)
      changed = true
      continue
    }
    if (entry.hidden) {
      entry.hidden = false
      changed = true
    }
  }

  return changed
}

function collapseTurnEntries(entries: TranscriptEntry[], turnId: number | null | undefined) {
  if (!turnId) {
    return { entries, nextId: entries.reduce((max, entry) => Math.max(max, entry.id), 0), changed: false }
  }

  const nextEntries = entries.map((entry) => ({ ...entry }))
  let nextId = nextEntries.reduce((max, entry) => Math.max(max, entry.id), 0)
  const promptIndex = nextEntries.findIndex((entry) => entry?.turnId === turnId && entry.role === "user")
  if (promptIndex === -1) {
    return { entries: nextEntries, nextId, changed: false }
  }

  for (let index = nextEntries.length - 1; index >= 0; index -= 1) {
    const entry = nextEntries[index]
    if (entry?.turnId === turnId && (entry.role === "turn_toggle" || entry.role === "turn_summary")) {
      nextEntries.splice(index, 1)
    }
  }

  const turnEntries = nextEntries.filter(
    (entry) => entry?.turnId === turnId && entry.role !== "turn_toggle" && entry.role !== "turn_summary",
  )
  const collapsibleEntries = turnEntries.filter((entry) => entry.role !== "user")
  const collapsedText = collapsedTurnText(collapsibleEntries)
  if (collapsibleEntries.length === 0 || !collapsedText) {
    return { entries: nextEntries, nextId, changed: false }
  }

  for (const entry of collapsibleEntries) {
    entry.hidden = true
  }
  nextEntries.splice(
    promptIndex + 1,
    0,
    {
      id: ++nextId,
      role: "turn_summary",
      text: collapsedText,
      turnId,
    },
    {
      id: ++nextId,
      role: "turn_toggle",
      text: "click to expand turn",
      turnId,
      toggleMode: "expand",
    },
  )

  return { entries: nextEntries, nextId, changed: true }
}

function expandTurnEntries(entries: TranscriptEntry[], turnId: number | null | undefined) {
  if (!turnId) {
    return { entries, nextId: entries.reduce((max, entry) => Math.max(max, entry.id), 0), changed: false }
  }

  const nextEntries = entries.map((entry) => ({ ...entry }))
  let nextId = nextEntries.reduce((max, entry) => Math.max(max, entry.id), 0)
  let changed = false

  for (const entry of nextEntries) {
    if (!entry || entry.turnId !== turnId) {
      continue
    }
    if (entry.role === "turn_toggle" || entry.role === "turn_summary") {
      changed = true
      continue
    }
    if (entry.role !== "user" && entry.hidden) {
      entry.hidden = false
      changed = true
    }
  }
  for (let index = nextEntries.length - 1; index >= 0; index -= 1) {
    const entry = nextEntries[index]
    if (entry?.turnId === turnId && (entry.role === "turn_toggle" || entry.role === "turn_summary")) {
      nextEntries.splice(index, 1)
    }
  }
  const insertIndex = nextEntries.reduce((lastIndex, entry, index) => {
    if (!entry || entry.turnId !== turnId) {
      return lastIndex
    }
    return index
  }, -1)
  if (changed && insertIndex >= 0) {
    nextEntries.splice(insertIndex + 1, 0, {
      id: ++nextId,
      role: "turn_toggle",
      text: "click to collapse turn",
      turnId,
      toggleMode: "collapse",
    })
  }

  return { entries: nextEntries, nextId, changed }
}

function findVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  toggleEntryId?: number,
) {
  if (!turnId) {
    return undefined
  }
  return entries.find((entry) => {
    if (!entry || entry.turnId !== turnId || entry.role !== "turn_toggle" || entry.hidden) {
      return false
    }
    return toggleEntryId === undefined || entry.id === toggleEntryId
  })
}

function normalizeTranscriptTurnIds(entries: TranscriptEntry[]) {
  let activeTurnId: number | undefined
  let nextTurnId = 1

  return entries.map((entry) => {
    const next: TranscriptEntry = { ...entry }
    if (entry.role === "user") {
      activeTurnId = entry.turnId ?? nextTurnId
      next.turnId = activeTurnId
      nextTurnId = Math.max(nextTurnId, activeTurnId + 1)
      return next
    }
    if (activeTurnId !== undefined) {
      next.turnId = activeTurnId
    }
    return next
  })
}

function collapsedTurnText(entries: TranscriptEntry[]) {
  const visibleEntries = entries.filter((entry) => isUserFacingTurnEntry(entry.role))
  if (visibleEntries.length > 0) {
    return visibleEntries.map((entry) => entry.text).join("")
  }
  return ""
}

function isUserFacingTurnEntry(role: TranscriptEntry["role"]) {
  return role === "assistant" || role === "error" || role === "notice"
}

type FooterFlash = {
  message: string
  tone: "info" | "error"
}

const OPEN_CONSOLE_ON_ERROR = (process.env.ARROBA_LOG_LEVEL ?? "").toLowerCase() === "debug"
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

  ensureTranscriptParsersRegistered()
  processLogger = createProcessLogger("cli")
  getLogger("cli.main")?.info("starting cli process", { argv })
  const options = parseArgs(argv)
  const preferences = await loadPreferences()
  if (options.model === "default") {
    options.model = preferences.providers?.opencode?.model ?? options.model
  }
  if (!options.effort.trim()) {
    options.effort = preferences.providers?.opencode?.effort ?? options.effort
  }
  const socketPath = options.socketPath ?? defaultSocketPath()
  const client = new LocalIpcClient(socketPath)
  const workspace = options.workspace ?? process.cwd()
  const worktree = options.worktree ?? workspace
  if (options.deleteSessionRef) {
    await deleteSessionByRef(client, options.deleteSessionRef, workspace)
    return
  }
  getLogger("cli.main")?.info("bootstrapping cli session", {
    socket_path: socketPath,
    workspace_id: workspace,
    worktree_id: worktree,
    client_id: options.clientId,
  })
  const bootstrap = await bootstrapSession(client, options, workspace, worktree, preferences, {
    logger: getLogger("cli.main"),
    listSessions,
    getProviderCatalog,
    createSession,
    resolveSession,
    attachToSession,
    getSessionState,
    launchProviderRun,
    tryGetProviderRun,
    catchUpAttachedSession,
    getSessionHistory,
    resolveVisibleAgentId: (session, nextPreferences) => {
      const focusedAgentId = session.focused_agent_id ?? session.agents[0]?.id ?? null
      return selectResponsePaneAgents(
        session.agents,
        focusedAgentId,
        sessionResponseLayout(session, nextPreferences.ui?.multiAgentResponseLayout) === "split",
      ).visibleTranscriptAgentId
    },
    prepareHistoryEntries: (entries, session) =>
      reindexTranscriptEntries(
        collapseHistoricalTurns(
          hydrateTranscriptEntries(entries),
          Boolean(session.active_prompt) || session.queued_prompts.length > 0,
        ),
        0,
      ),
  })
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

function ensureTranscriptParsersRegistered() {
  if (transcriptParsersRegistered) {
    return
  }
  addDefaultParsers(parserConfig.parsers)
  transcriptParsersRegistered = true
}

function ArrobaCliApp(props: { bootstrap: BootstrapState }) {
  const { client, options } = props.bootstrap
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
  const initialPreferences = props.bootstrap.preferences
  const [sessionState, setSessionState] = createSignal(initialSession)
  const [attachmentState, setAttachmentState] = createSignal<RuntimeAttachment | null>(initialBinding?.attachment ?? null)
  const [providerRunState, setProviderRunState] = createSignal<RuntimeProviderRun | null>(initialBinding?.providerRun ?? null)
  const [createdSessionState, setCreatedSessionState] = createSignal(initialBinding?.createdSession ?? false)
  const [availableSessions, setAvailableSessions] = createSignal<RuntimeSession[]>(initialSessions)
  const [providerCatalogState, setProviderCatalogState] = createSignal<ProviderCatalog>(initialProviderCatalog)
  const [waitingRoomState, setWaitingRoomState] = createSignal<WaitingRoomState>(
    createWaitingRoomState(initialSessions, initialProviderCatalog, options.model, options.effort),
  )
  const [commandCenterQuery, setCommandCenterQuery] = createSignal("")
  const [commandCenterItems, setCommandCenterItems] = createSignal<CommandCenterItem[]>([])
  const [commandCenterIndex, setCommandCenterIndex] = createSignal(0)
  const [centerMode, setCenterMode] = createSignal<"transcript" | "tree">("transcript")
  const [multiAgentResponseLayout, setMultiAgentResponseLayout] = createSignal<MultiAgentResponseLayout>(
    sessionResponseLayout(initialSession, initialPreferences.ui?.multiAgentResponseLayout),
  )
  const [directoryTreeState, setDirectoryTreeState] = createSignal<DirectoryTreeState | null>(null)
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
  const [nextHistoryCursor, setNextHistoryCursor] = createSignal<SessionHistoryCursor | null>(initialBinding?.nextHistoryCursor ?? null)
  const [agentPanePreviews, setAgentPanePreviews] = createSignal<Record<string, string>>({})
  const [agentPaneEntries, setAgentPaneEntries] = createSignal<Record<string, TranscriptEntry[]>>({})
  const [loadingHistory, setLoadingHistory] = createSignal(false)
  const [workingAnimationFrame, setWorkingAnimationFrame] = createSignal(0)
  const [working, setWorking] = createSignal(Boolean(initialSession.active_prompt) || initialSession.queued_prompts.length > 0)
  const [footerFlash, setFooterFlash] = createSignal<FooterFlash | null>(null)
  const [pendingAttachments, setPendingAttachments] = createSignal<PendingPromptAttachment[]>([])
  const [hotkeysOpen, setHotkeysOpen] = createSignal(false)
  const [expandedTurnIdsByAgent, setExpandedTurnIdsByAgent] = createSignal<Record<string, number[]>>({})
  let stopRequestInFlight = false
  let promptInput: TextareaRenderable | undefined
  let hotkeysFocus: Renderable | null = null
  let transcriptScrollbox: ScrollBoxRenderable | undefined
  let responseLayoutBox: BoxRenderable | undefined
  let responseTopRowBox: BoxRenderable | undefined
  let responsePrimaryPane: BoxRenderable | undefined
  let responseSecondaryPane: BoxRenderable | undefined
  let responseSecondaryScrollbox: ScrollBoxRenderable | undefined
  let responseTertiaryPane: BoxRenderable | undefined
  let responseTertiaryScrollbox: ScrollBoxRenderable | undefined
  let responsePrimaryFooterBox: BoxRenderable | undefined
  let responseSecondaryFooterBox: BoxRenderable | undefined
  let responseTertiaryFooterBox: BoxRenderable | undefined
  let responsePrimaryFooterText: TextRenderable | undefined
  let responseSecondaryFooterText: TextRenderable | undefined
  let responseTertiaryFooterText: TextRenderable | undefined
  let responsePrimaryFooterBadgeTexts: TextRenderable[] = []
  let responseSecondaryFooterBadgeTexts: TextRenderable[] = []
  let responseTertiaryFooterBadgeTexts: TextRenderable[] = []
  let responseSecondaryAgentId: string | null = null
  let responseTertiaryAgentId: string | null = null
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
  const transcriptSyntax = createTranscriptSyntaxStyle()
  let emptyTranscriptRenderable: BoxRenderable | undefined
  let footerFlashTimeout: ReturnType<typeof startTimeout> | undefined
  let lastTranscriptScrollTop = 0
  let historyLoadGeneration = 0
  let pendingHistoryScrollRestore = 0
  let pendingSessionChromeUpdate = false
  let pendingTranscriptRender = false
  let pendingResponsePaneRepaint = 0
  let pendingSplitPaneRefresh = 0
  let uiBatchDepth = 0
  let pendingTerminalRecordFlush: ReturnType<typeof startTimeout> | undefined
  let pendingTerminalRecords: TerminalOutputRecord[] = []
  let pendingTurnCompletion: ReturnType<typeof startTimeout> | undefined
  // Connection resilience tracking
  let lastDaemonActivityAt = Date.now()
  let connectionWatchdogTimeout: ReturnType<typeof startTimeout> | undefined
  let consecutiveSilentPolls = 0
  const SILENT_POLL_THRESHOLD = 8 // ~2 seconds of no activity (8 * 250ms polling interval)
  let providerRecoveryInFlight = false
  let currentTurnId = computeCurrentTurnId(initialEntries)
  let nextTurnId = computeNextTurnId(initialEntries)
  let promptTextSnapshot = ""
  let promptTextMuting = false
  let promptDropPending = false

  const isAttached = () => attachmentState() !== null
  const focusedAgentId = () => sessionState().focused_agent_id ?? sessionState().agents[0]?.id ?? null
  const multiAgentMode = () => isAttached() && sessionState().agents.length > 1
  const splitAgentResponseMode = () => isAttached() && multiAgentResponseLayout() === "split"
  const responsePaneSelection = createMemo(() => selectResponsePaneAgents(
    sessionState().agents,
    focusedAgentId(),
    splitAgentResponseMode(),
  ))
  const responsePrimaryAgent = () => responsePaneSelection().primary
  const responseSecondaryAgent = () => responsePaneSelection().secondary
  const responseTertiaryAgent = () => responsePaneSelection().tertiary
  const visibleTranscriptAgentId = () => responsePaneSelection().visibleTranscriptAgentId
  const primaryTranscriptSurfaceTone = () => resolveTranscriptSurfaceTone(splitAgentResponseMode(), responsePrimaryAgent()?.id === focusedAgentId())
  const auxiliaryTranscriptSurfaceTone = (agentId: string | null | undefined) => {
    return resolveTranscriptSurfaceTone(splitAgentResponseMode(), Boolean(agentId) && agentId === focusedAgentId())
  }
  const scheduleResponsePaneRepaint = () => {
    const repaintToken = ++pendingResponsePaneRepaint
    const repaint = () => {
      if (repaintToken !== pendingResponsePaneRepaint) {
        return
      }
      const seen = new Set<string | number>()
      requestRenderableTreeRender(responseLayoutBox, seen)
      requestRenderableTreeRender(historyLoadingBox, seen)
      ;(renderer as { requestRender?: () => void }).requestRender?.()
    }
    repaint()
    startTimeout(repaint, 0)
    startTimeout(repaint, 16)
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
  const agentPanePreview = (agentId: string) => agentPanePreviews()[agentId] ?? ""
  const agentActivityLabel = (agentId: string | null | undefined) => (agentId ? agentActivityLabels()[agentId] ?? null : null)
  const focusedAgent = () => sessionState().agents.find((agent) => agent.id === focusedAgentId()) ?? null
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
  const shouldPreserveAgentActivityLabel = (agentId: string | null | undefined) => {
    if (!agentId) {
      return false
    }
    return streamingAgentId() === agentId
      || activePrompt()?.target_agent_id === agentId
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
  const queueDepth = () => sessionState().queued_prompts.length
  const connectedClientCount = () => sessionState().attachment_ids.length
  const activePrompt = () => sessionState().active_prompt
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
    focusedAgent: focusedAgent(),
    focusedAgentActivityLabel: agentActivityLabel(focusedAgent()?.id),
    streamingAgentId: streamingAgentId(),
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
    appLogger?.info(`view debug: ${phase}`, {
      layout: multiAgentResponseLayout(),
      layout_is_split: multiAgentResponseLayout() === "split",
      split_active: splitAgentResponseMode(),
      attached: isAttached(),
      center_mode: centerMode(),
      agent_count: sessionState().agents.length,
      focused_agent_id: focusedAgentId(),
      has_transcript_scrollbox: Boolean(transcriptScrollbox),
      ...fields,
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
    const update = deriveWaitingRoomStateUpdate({
      currentState: waitingRoomState(),
      nextState: next,
      sessions: availableSessions(),
      catalog: providerCatalogState(),
      currentModel: options.model,
    })
    setWaitingRoomState(update.normalizedState)
    options.model = update.nextModel
    options.effort = update.nextEffort
    if (update.shouldPersistProviderPreferences) {
      void saveProviderPreferences("opencode", {
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
    const decision = deriveWaitingRoomActivationDecision({
      state: waitingRoomState(),
      sessions: availableSessions(),
      catalog: providerCatalogState(),
      currentModel: options.model,
    })
    if (decision.action === "create") {
      const root = options.workspace ?? process.cwd()
      const session = await createSession(client, root, options.worktree ?? root)
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
  }
  const refreshWaitingRoomData = async () => {
    const [sessions, catalog] = await Promise.all([
      listSessions(client),
      getProviderCatalog(client, appLogger),
    ])
    setAvailableSessions(sessions)
    setProviderCatalogState(catalog)
    reconcileWaitingRoom(waitingRoomState())
  }
  const applyModelSelection = async (modelId: string) => {
    const decision = deriveWaitingRoomModelSelectionDecision({
      modelId,
      state: waitingRoomState(),
      sessions: availableSessions(),
      catalog: providerCatalogState(),
      configuredEffort: options.effort,
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
    const run = await launchProviderRun(
      client,
      sessionState().id,
      options.accountProfile,
      decision.launch.model,
      decision.launch.effort,
      focusedAgentId(),
    )
    setProviderRunState(run)
    applySessionState(await getSessionState(client, sessionState().id))
    await maybeResize(client, sessionState().id)
    flashFooter(`model set to ${decision.selectedModelId}`, "info")
  }
  const applyVariantSelection = async (variant: string) => {
    const decision = deriveWaitingRoomVariantSelectionDecision({
      variant,
      currentModelId: currentModelId(),
      state: waitingRoomState(),
      sessions: availableSessions(),
      catalog: providerCatalogState(),
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
    const run = await launchProviderRun(
      client,
      sessionState().id,
      options.accountProfile,
      decision.launch.model,
      decision.launch.effort,
      focusedAgentId(),
    )
    setProviderRunState(run)
    applySessionState(await getSessionState(client, sessionState().id))
    await maybeResize(client, sessionState().id)
    flashFooter(`variant set to ${decision.selectedVariant}`, "info")
  }
  const loadDirectoryTree = async (treePath = "") => {
    const attachment = attachmentState()
    if (!attachment) {
      return
    }
    const response = await client.send<Record<string, unknown>>(readDirectoryTreeRequest(sessionState().id, attachment.id, treePath || null, 1))
    const payload = expectVariant<{ result: ReadDirectoryTreeResult }>(response, "DirectoryTreeRead")
    const next = treePath && directoryTreeState()
      ? mergeDirectoryTreeEntries(directoryTreeState()!, treePath, payload.result.entries)
      : createDirectoryTreeState(payload.result.root_path, payload.result.entries)
    const previous = directoryTreeState()
    if (previous && previous.rootPath === next.rootPath) {
      next.expandedPaths = previous.expandedPaths.filter((value) => value === "" || next.entries.some((entry) => entry.relative_path === value))
      next.selectedPath = next.entries.some((entry) => entry.relative_path === previous.selectedPath) || previous.selectedPath === ""
        ? previous.selectedPath
        : next.selectedPath
    }
    setDirectoryTreeState(next)
  }
  const toggleCenterMode = async () => {
    if (!isAttached()) {
      return
    }
    if (centerMode() === "tree") {
      setCenterMode("transcript")
      rebuildTranscript()
      if (transcriptScrollbox) {
        const maxScrollTop = Math.max(0, transcriptScrollbox.scrollHeight - transcriptScrollbox.height)
        const target = Math.max(0, Math.min(lastTranscriptScrollTop, maxScrollTop))
        transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: target })
        transcriptScrollbox.requestRender()
      }
      return
    }
    lastTranscriptScrollTop = transcriptScrollbox?.scrollTop ?? lastTranscriptScrollTop
    try {
      await loadDirectoryTree()
    } catch (error) {
      flashFooter(`failed to load tree: ${formatError(error)}`, "error")
      return
    }
    setCenterMode("tree")
    rebuildTranscript()
  }
  const navigateDirectoryTree = (direction: "up" | "down") => {
    const state = directoryTreeState()
    if (!state) {
      return
    }
    setDirectoryTreeState(moveDirectoryTreeSelection(state, direction))
    rebuildTranscript()
  }
  const activateDirectoryTreeSelection = () => {
    const state = directoryTreeState()
    if (!state) {
      return
    }
    const row = buildDirectoryTreeRows(state).find((candidate) => candidate.id === state.selectedPath)
    if (!row || (row.kind !== "root" && row.kind !== "directory")) {
      return
    }
    const applyToggle = () => {
      setDirectoryTreeState((current) => (current ? toggleDirectoryTreeExpansion(current) : current))
      rebuildTranscript()
    }
    if (row.id === "" || isDirectoryTreePathLoaded(state, row.id)) {
      applyToggle()
      return
    }
    void loadDirectoryTree(row.id)
      .then(() => {
        applyToggle()
      })
      .catch((error) => {
        flashFooter(`failed to load tree: ${formatError(error)}`, "error")
      })
  }
  const currentProviderSelection = () => deriveCurrentProviderSelection({
    providerRun: providerRunState(),
    waitingRoomState: waitingRoomState(),
    defaultModel: options.model,
    defaultEffort: options.effort,
  })
  const promptMetaParts = (): PromptMetaPart[] => derivePromptMetaState({
    providerRun: providerRunState(),
    waitingRoomState: waitingRoomState(),
    defaultModel: options.model,
    defaultEffort: options.effort,
  })
  const promptUsageMeta = () => derivePromptUsageState({
    providerRun: providerRunState(),
    catalog: providerCatalogState(),
  })
  const currentModelId = () => currentProviderSelection().model
  const currentVariantId = () => currentProviderSelection().effort
  const syncCommandCenter = (value = promptInput?.plainText ?? promptTextSnapshot) => {
    setCommandCenterQuery(value)
    const items = buildCommandCenterItems(value, {
      providerCatalog: providerCatalogState(),
      currentModel: currentModelId(),
      currentVariant: currentVariantId(),
    })
    setCommandCenterItems(items)
    setCommandCenterIndex((index) => (items.length === 0 ? 0 : Math.max(0, Math.min(index, items.length - 1))))
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
    if (event.name === "return" || event.name === "enter" || event.name === "tab") {
      const item = selectedCommandCenterItem()
      if (!item) {
        return false
      }
      event.preventDefault?.()
      event.stopPropagation?.()
      void selectCommandCenterItem(item)
      return true
    }
    return false
  }
  const selectCommandCenterFromSubmit = () => {
    const item = selectedCommandCenterItem()
    if (!item) {
      return false
    }
    // If the item ends with space and the prompt already contains this command,
    // close the command center and let submitPrompt handle the actual execution
    if (item.value.endsWith(" ")) {
      const currentPrompt = promptInput?.plainText ?? ""
      if (currentPrompt.startsWith(item.value) || currentPrompt === item.value.trim()) {
        clearCommandCenter()
        syncCommandCenter("")
        return false
      }
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

    for (const [index, item] of commandCenterItems().entries()) {
      const selected = index === commandCenterIndex()
      const row = new BoxRenderable(renderer, {
        flexDirection: "row",
        justifyContent: "space-between",
        paddingLeft: 1,
        paddingRight: 1,
        ...(selected ? { backgroundColor: theme.primary } : {}),
      })
      row.add(new TextRenderable(renderer, {
        content: item.label,
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

    commandCenterBox.add(panel)
    commandCenterBox.requestRender()
  }
  const cancelPendingTurnCompletion = () => {
    if (pendingTurnCompletion) {
      clearTimeout(pendingTurnCompletion)
      pendingTurnCompletion = undefined
    }
  }
  const collapseCompletedTurns = () => {
    replaceTranscriptEntries(collapseHistoricalTurns(entries.filter(Boolean).map((entry) => ({ ...entry })), false))
    for (const agent of sessionState().agents) {
      setAgentTranscriptEntries(
        agent.id,
        trimAgentPaneEntries({
          entries: collapseHistoricalTurns(currentAgentPaneEntries(agent.id), false),
          maxEntries: LIVE_TRANSCRIPT_LIMIT,
          maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
          onTrimmedMergeKey: (mergeKey) => {
            auxiliaryAgentPaneTools(agent.id).delete(mergeKey)
          },
        }),
      )
    }
  }
  const finalizeTurnCompletion = () => {
    cancelPendingTurnCompletion()
    if (sessionHasPromptWork(sessionState()) || pendingTerminalRecords.length > 0 || pendingTerminalRecordFlush) {
      return
    }
    batch(() => {
      collapseCompletedTurns()
      activeToolLabels.clear()
      setSubmitting(false)
      setProviderActivityLabel(null)
      setActiveStatusLabel(null)
      if (!activePrompt() && statusLine() === "Cancellation requested.") {
        setStatusLine(DEFAULT_CONNECTED_STATUS)
      }
      setWorking(false)
    })
    updateSessionChrome()
  }
  const scheduleTurnCompletion = () => {
    cancelPendingTurnCompletion()
    if (sessionHasPromptWork(sessionState()) || pendingTerminalRecords.length > 0 || pendingTerminalRecordFlush) {
      return
    }
    pendingTurnCompletion = startTimeout(() => {
      pendingTurnCompletion = undefined
      finalizeTurnCompletion()
    }, TURN_COMPLETION_SETTLE_MS)
  }
  const sessionStatusMode = (): SessionStatusMode => {
    return deriveSessionStatusMode({
      daemonDisconnected: daemonDisconnected(),
      working: working(),
      hasActivePrompt: Boolean(activePrompt()),
      submitting: submitting(),
      queueDepth: queueDepth(),
    })
  }
  const footerHint = () => {
    return deriveFooterHint({
      fatalError: fatalError(),
      activePromptId: activePrompt()?.id ?? null,
      queueDepth: queueDepth(),
      statusLine: statusLine(),
    })
  }
  const promptPlaceholder = () => (isAttached() ? ATTACHED_PROMPT_PLACEHOLDER : SESSION_NEW_PLACEHOLDER)
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
    promptTextSnapshot = value
    refreshPromptAttachmentHighlights()
    promptTextMuting = false
  }
  const syncPromptPlaceholder = () => {
    if (!promptInput) {
      return
    }
    promptInput.placeholder = promptPlaceholder()
  }
  const clearPendingPromptAttachments = () => {
    setPendingAttachments([])
    refreshPromptAttachmentHighlights()
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }
  const attachmentTokenKind = (kind: PromptAttachmentKind) => (kind === "image" ? "image" : kind === "pdf" ? "pdf" : "file")
  const hotkeySections = (): HotkeySection[] => [
    { title: "Global", items: GLOBAL_HOTKEYS },
    isAttached()
      ? { title: "Session", items: SESSION_HOTKEYS }
      : { title: "Waiting room", items: WAITING_ROOM_HOTKEYS },
  ]
  const renderHotkeysOverlay = () => {
    if (!hotkeysOverlayBox) {
      return
    }
    for (const child of [...hotkeysOverlayBox.getChildren()]) {
      hotkeysOverlayBox.remove(child.id)
      child.destroyRecursively()
    }
    if (!hotkeysOpen()) {
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
      closeHotkeys()
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
      setPromptText(`${text.slice(0, edit.start)}${text.slice(edit.end)}`)
      promptInput.cursorOffset = cursor
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
    if (promptTextMuting || promptDropPending) {
      promptTextSnapshot = value
      syncCommandCenter(value)
      return
    }
    const drop = extractDroppedPromptAttachments(promptTextSnapshot, value, process.cwd())
    if (!drop) {
      syncPendingPromptAttachmentsFromText(value)
      promptTextSnapshot = value
      syncCommandCenter(value)
      return
    }
    setPromptText(drop.nextText)
    syncCommandCenter(drop.nextText)
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

  const collapseTurn = (turnId: number | null | undefined, toggleEntryId?: number) => {
    if (!turnId) {
      return
    }
    const agentId = visibleTranscriptAgentId()
    const scrollbox = transcriptScrollbox
    const previousScrollTop = scrollbox?.scrollTop ?? 0
    const previousViewportHeight = scrollbox?.height ?? 0
    const previousVisibleTurnHeight = measureVisibleTurnHeight(turnId)
    const { entries: nextEntries, nextId, changed } = collapseTurnEntries(entries.filter(Boolean), turnId)
    setEntries(reconcile(nextEntries))
    setEntryCounter(nextId)
    if (changed) {
      setExpandedTurnState(agentId, turnId, false)
      persistVisibleTranscriptEntries(nextEntries)
    }
    // Always rebuild transcript even if nothing collapsed (to ensure UI consistency)
    rebuildTranscript()
    if (scrollbox && changed) {
      restoreTurnScrollPosition(
        scrollbox,
        turnId,
        previousScrollTop,
        previousViewportHeight,
        previousVisibleTurnHeight,
      )
    }
  }

  const expandTurn = (turnId: number | null | undefined) => {
    if (!turnId) {
      return
    }
    const agentId = visibleTranscriptAgentId()
    const scrollbox = transcriptScrollbox
    const previousScrollTop = scrollbox?.scrollTop ?? 0
    const previousViewportHeight = scrollbox?.height ?? 0
    const previousVisibleTurnHeight = measureVisibleTurnHeight(turnId)
    const { entries: nextEntries, nextId, changed } = expandTurnEntries(entries.filter(Boolean), turnId)
    setEntries(reconcile(nextEntries))
    if (!changed) {
      return
    }
    setExpandedTurnState(agentId, turnId, true)
    setEntryCounter(nextId)
    persistVisibleTranscriptEntries(nextEntries)
    rebuildTranscript()
    if (scrollbox) {
      restoreTurnScrollPosition(
        scrollbox,
        turnId,
        previousScrollTop,
        previousViewportHeight,
        previousVisibleTurnHeight,
      )
    }
  }

  const toggleTurn = (turnId: number | null | undefined, toggleEntryId?: number) => {
    if (!turnId) {
      return
    }
    const toggleEntry = findVisibleTurnToggle(entries.filter(Boolean), turnId, toggleEntryId)
    if (toggleEntryId !== undefined && !toggleEntry) {
      return
    }
    if (toggleEntry?.toggleMode === "expand") {
      expandTurn(turnId)
      return
    }
    collapseTurn(turnId, toggleEntryId)
  }

  const appendEntry = (entry: Omit<TranscriptEntry, "id">) => {
    const nextId = entryCounter() + 1
    const nextEntry: TranscriptEntry = { id: nextId, ...entry }
    if (nextEntry.turnId === undefined && currentTurnId !== null) {
      nextEntry.turnId = currentTurnId
    }
    setEntryCounter(nextId)
    setEntries(entries.length, nextEntry)
    mountTranscriptEntry(nextEntry)
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

  const appendUserPrompt = (text: string, agentId?: string | null) => {
    const targetAgentId = agentId ?? focusedAgentId()
    if (splitAgentResponseMode() && targetAgentId && targetAgentId !== responsePrimaryAgent()?.id) {
      const paneEntries = agentPaneEntries()[targetAgentId] ?? []
      appendTranscriptEntryToAgentPane(targetAgentId, {
        role: "user",
        text: trimSingleTrailingNewline(text),
        turnId: computeNextTurnId(paneEntries),
      })
      setSubmitting(true)
      setWorking(true)
      updateSessionChrome()
      return
    }
    collapseTurn(currentTurnId)
    const turnId = nextTurnId
    nextTurnId += 1
    currentTurnId = turnId
    appendEntry({ role: "user", text: trimSingleTrailingNewline(text), turnId })
    syncVisibleTranscriptPreview()
    setSubmitting(true)
    setWorking(true)
    updateSessionChrome()
    scrollTranscriptToBottom()
  }

  const appendNotice = (text: string, emphasis: TranscriptEntry["emphasis"] = "muted") => {
    appendEntry({ role: "notice", text, emphasis })
    syncVisibleTranscriptPreview()
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
    appendEntry({ role: "error", text: normalized, emphasis: "error" })
    syncVisibleTranscriptPreview()
    updateSessionChrome()
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
    }, 3_000)
  }

  const hotkeyDebug = (message: string) => {
    appLogger?.debug("hotkeys footer debug", { detail: message })
    if ((process.env.ARROBA_LOG_LEVEL ?? "").toLowerCase() !== "debug") {
      return
    }
    flashFooter(`[hotkeys] ${message}`, "info")
  }

  const copySelection = () => {
    const text = renderer.getSelection()?.getSelectedText()
    renderer.clearSelection()
    if (!text) {
      return
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
  }

  const applySessionState = (nextSession: RuntimeSession) => {
    const previousFocusedAgentId = focusedAgentId()
    const previousLayout = multiAgentResponseLayout()
    const transition = deriveSessionTransitionState({
      currentSession: sessionState(),
      nextSession,
      currentWorking: working(),
      currentStreamingAgentId: streamingAgentId(),
      currentAgentActivityLabels: agentActivityLabels(),
      layoutPreference: initialPreferences.ui?.multiAgentResponseLayout,
    })
    setSessionState(nextSession)
    setAgentActivityLabels(transition.nextAgentActivityLabels)
    setStreamingAgentId(transition.nextStreamingAgentId)
    setMultiAgentResponseLayout(transition.nextLayout)
    setWorking(transition.nextWorking)
    if (transition.nextHasPromptWork) {
      cancelPendingTurnCompletion()
    } else {
      scheduleTurnCompletion()
    }
    setProviderActivityLabel(transition.nextFocusedActivityLabel)
    setActiveStatusLabel(transition.nextFocusedActivityLabel)
    if (!nextSession.active_prompt) {
      setSubmitting(false)
      stopRequestInFlight = false
    }
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

  const applyProviderActivity = (active: boolean) => {
    if (active) {
      cancelPendingTurnCompletion()
      setWorking(true)
    } else {
      scheduleTurnCompletion()
    }
    updateSessionChrome()
  }

  const syncVisibleActivityLabel = () => {
    setActiveStatusLabel(deriveVisibleActivityLabel({
      providerActivityLabel: providerActivityLabel(),
      activeToolLabels: activeToolLabels.values(),
    }))
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
    let mergedEntryId: number | undefined
    let mergedText: string | undefined
    let nextEntry: TranscriptEntry | undefined
    const nextId = entryCounter() + 1
    setEntries(
      produce((draft) => {
        expandLatestTurnForLiveUpdateInPlace(draft)
        if (mergeKey) {
          let existing: TranscriptEntry | undefined
          for (let index = draft.length - 1; index >= 0; index -= 1) {
            const candidate = draft[index]
            if (candidate?.role === role && candidate.mergeKey === mergeKey) {
              existing = candidate
              break
            }
          }
          if (existing) {
            if (role === "assistant" || role === "reasoning") {
              existing.text += normalized
              if (normalizedSource !== undefined) {
                existing.sourceText = `${existing.sourceText ?? ""}${normalizedSource}`
              }
            } else {
              existing.text = normalized
              if (normalizedSource !== undefined) existing.sourceText = normalizedSource
            }
            mergedEntryId = existing.id
            mergedText = existing.text
            return
          }
        }
        const last = draft.at(-1)
        if (!mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
          last.text += normalized
          mergedEntryId = last.id
          mergedText = last.text
          return
        }
        nextEntry = {
          id: nextId,
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
        draft.push(nextEntry)
      }),
    )
    if (mergedEntryId !== undefined && mergedText !== undefined) {
      updateTranscriptEntry(mergedEntryId, mergedText, normalizedSource)
      enforceTranscriptRetention()
      return
    }
    if (!nextEntry) {
      return
    }
    setEntryCounter(nextId)
    mountTranscriptEntry(nextEntry)
    enforceTranscriptRetention()
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
    recordDaemonActivity("terminal_record")
    const text = Buffer.from(record.bytes).toString("utf8")
    const recordAgentId = resolveTerminalRecordAgentId(record)
    if (recordAgentId && record.kind !== "prompt_echo") {
      setStreamingAgentId(recordAgentId)
    }
    if (splitAgentResponseMode() && recordAgentId) {
      if (record.kind === "provider_status") {
        const activityLabel = getProviderActivityLabel(text)
        setAgentActivityLabel(recordAgentId, activityLabel)
        if (recordAgentId === focusedAgentId()) {
          const nextFocusedActivityLabel = activityLabel ?? agentActivityLabel(recordAgentId)
          setProviderActivityLabel(nextFocusedActivityLabel)
          applyProviderActivity(nextFocusedActivityLabel !== null)
          if (activityLabel !== null) {
            syncVisibleActivityLabel()
          }
        }
      }
      switch (record.kind) {
        case "prompt_echo": {
          if (hasTrailingUserPrompt(recordAgentId, text)) {
            break
          }
          const paneEntries = currentAgentPaneEntries(recordAgentId)
          appendTranscriptEntryToAgentPane(recordAgentId, {
            role: "user",
            text: trimSingleTrailingNewline(text),
            turnId: computeNextTurnId(paneEntries),
          })
          break
        }
        case "provider_reasoning":
          appendProviderChunkToAgentPane(recordAgentId, "reasoning", text, record.merge_key)
          break
        case "provider_tool":
          appendToolUpdateToAgentPane(recordAgentId, text)
          break
        case "provider_error": {
          const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
          if (normalized) {
            appendTranscriptEntryToAgentPane(recordAgentId, { role: "error", text: normalized, emphasis: "error" })
          }
          break
        }
        case "provider_status":
          if (shouldRenderProviderStatus(text)) {
            appendProviderChunkToAgentPane(recordAgentId, "status", text, "__provider_status__")
          }
          break
        default:
          appendProviderChunkToAgentPane(recordAgentId, "assistant", text, record.merge_key)
          break
      }
      return
    }
    const mainTranscriptAgentId = visibleTranscriptAgentId()
    const isVisibleRecord = !recordAgentId || recordAgentId === mainTranscriptAgentId
    if (!isVisibleRecord) {
      if (recordAgentId) {
        switch (record.kind) {
          case "prompt_echo": {
            if (hasTrailingUserPrompt(recordAgentId, text)) {
              break
            }
            const paneEntries = currentAgentPaneEntries(recordAgentId)
            appendTranscriptEntryToAgentPane(recordAgentId, {
              role: "user",
              text: trimSingleTrailingNewline(text),
              turnId: computeNextTurnId(paneEntries),
            })
            break
          }
          case "provider_reasoning":
            appendProviderChunkToAgentPane(recordAgentId, "reasoning", text, record.merge_key)
            break
          case "provider_tool":
            appendToolUpdateToAgentPane(recordAgentId, text)
            break
          case "provider_error": {
            const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
            if (normalized) {
              appendTranscriptEntryToAgentPane(recordAgentId, { role: "error", text: normalized, emphasis: "error" })
          }
          break
        }
        case "provider_status":
          setAgentActivityLabel(recordAgentId, getProviderActivityLabel(text))
          if (shouldRenderProviderStatus(text)) {
            appendProviderChunkToAgentPane(recordAgentId, "status", text, "__provider_status__")
          }
          break
          default:
            appendProviderChunkToAgentPane(recordAgentId, "assistant", text, record.merge_key)
            break
        }
        return
      }
      appendAgentPanePreview(recordAgentId, previewLineForTerminalRecord(record.kind, text))
      return
    }
    switch (record.kind) {
      case "prompt_echo":
        appendEntry({ role: "user", text: trimSingleTrailingNewline(text) })
        syncVisibleTranscriptPreview()
        break
      case "provider_reasoning":
        appendProviderChunk("reasoning", text, record.merge_key)
        syncVisibleTranscriptPreview()
        break
      case "provider_tool":
        appendToolUpdate(text)
        syncVisibleTranscriptPreview()
        break
      case "provider_error":
        appendProviderError(text)
        syncVisibleTranscriptPreview()
        break
      case "provider_status": {
        const activityLabel = getProviderActivityLabel(text)
        setAgentActivityLabel(recordAgentId, activityLabel)
        const nextFocusedActivityLabel = activityLabel ?? agentActivityLabel(recordAgentId)
        setProviderActivityLabel(nextFocusedActivityLabel)
        applyProviderActivity(nextFocusedActivityLabel !== null)
        if (activityLabel !== null) {
          syncVisibleActivityLabel()
        }
        if (shouldRenderProviderStatus(text)) {
          appendProviderChunk("status", text, "__provider_status__")
          syncVisibleTranscriptPreview()
        }
        break
      }
      default:
        appendProviderChunk("assistant", text, record.merge_key)
        syncVisibleTranscriptPreview()
        break
    }
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
    infoText: TextRenderable | undefined,
    badgeTexts: TextRenderable[],
    assignInfoText: (value: TextRenderable) => void,
    assignBadgeTexts: (value: TextRenderable[]) => void,
  ) => {
    if (!footerBox || infoText) {
      return
    }
    footerBox.flexDirection = "row"
    footerBox.gap = 1
    const badgeBox = new BoxRenderable(renderer, {
      flexDirection: "row",
      flexShrink: 0,
    })
    const nextBadgeTexts = Array.from({ length: STATUS_BADGE_WIDTH }, () => new TextRenderable(renderer, { wrapMode: "none" }))
    for (const text of nextBadgeTexts) {
      badgeBox.add(text)
    }
    const nextInfoText = new TextRenderable(renderer, { fg: theme.textMuted, wrapMode: "none" })
    footerBox.add(badgeBox)
    footerBox.add(nextInfoText)
    assignInfoText(nextInfoText)
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
    for (let index = 0; index < STATUS_BADGE_WIDTH; index += 1) {
      const character = label[index] ?? " "
      let fg = theme.success
      if (tone === "disconnected" || tone === "error") {
        fg = theme.error
      } else if (tone === "working") {
        const distance = reflectedDistance(index, label.length, workingAnimationFrame())
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

  const renderSplitPaneFooters = () => {
    const showSplitFooters = splitAgentResponseMode() && sessionState().agents.length > 1
    ensureSplitPaneFooterRenderables(
      responsePrimaryFooterBox,
      responsePrimaryFooterText,
      responsePrimaryFooterBadgeTexts,
      (value) => {
        responsePrimaryFooterText = value
      },
      (value) => {
        responsePrimaryFooterBadgeTexts = value
      },
    )
    ensureSplitPaneFooterRenderables(
      responseSecondaryFooterBox,
      responseSecondaryFooterText,
      responseSecondaryFooterBadgeTexts,
      (value) => {
        responseSecondaryFooterText = value
      },
      (value) => {
        responseSecondaryFooterBadgeTexts = value
      },
    )
    ensureSplitPaneFooterRenderables(
      responseTertiaryFooterBox,
      responseTertiaryFooterText,
      responseTertiaryFooterBadgeTexts,
      (value) => {
        responseTertiaryFooterText = value
      },
      (value) => {
        responseTertiaryFooterBadgeTexts = value
      },
    )

    if (!showSplitFooters) {
      renderStatusBadgeTexts(responsePrimaryFooterBadgeTexts, "", "idle")
      renderStatusBadgeTexts(responseSecondaryFooterBadgeTexts, "", "idle")
      renderStatusBadgeTexts(responseTertiaryFooterBadgeTexts, "", "idle")
      setTextRenderable(responsePrimaryFooterText, "", theme.textMuted)
      setTextRenderable(responseSecondaryFooterText, "", theme.textMuted)
      setTextRenderable(responseTertiaryFooterText, "", theme.textMuted)
      responsePrimaryFooterBox?.requestRender()
      responseSecondaryFooterBox?.requestRender()
      responseTertiaryFooterBox?.requestRender()
      return
    }

    const providerRun = providerRunState()
    const fallbackModel = providerRun?.model ?? null
    const fallbackVariant = providerRun?.variant ?? null
    const footerState = buildSplitPaneFooterState({
      mode: sessionStatusMode(),
      selection: responsePaneSelection(),
      focusedAgentId: focusedAgentId(),
      streamingAgentId: streamingAgentId(),
      activityLabels: agentActivityLabels(),
      catalog: providerCatalogState(),
      fallbackModel,
      fallbackVariant,
    })

    renderStatusBadgeTexts(responsePrimaryFooterBadgeTexts, footerState.primary.badge.label, footerState.primary.badge.tone)
    renderStatusBadgeTexts(responseSecondaryFooterBadgeTexts, footerState.secondary.badge.label, footerState.secondary.badge.tone)
    renderStatusBadgeTexts(responseTertiaryFooterBadgeTexts, footerState.tertiary.badge.label, footerState.tertiary.badge.tone)
    setTextRenderable(
      responsePrimaryFooterText,
      footerState.primary.info,
      footerState.primary.focused ? theme.text : theme.textMuted,
      footerState.primary.focused ? TextAttributes.BOLD : TextAttributes.NONE,
    )
    setTextRenderable(
      responseSecondaryFooterText,
      footerState.secondary.info,
      footerState.secondary.focused ? theme.text : theme.textMuted,
      footerState.secondary.focused ? TextAttributes.BOLD : TextAttributes.NONE,
    )
    setTextRenderable(
      responseTertiaryFooterText,
      footerState.tertiary.info,
      footerState.tertiary.focused ? theme.text : theme.textMuted,
      footerState.tertiary.focused ? TextAttributes.BOLD : TextAttributes.NONE,
    )
    responsePrimaryFooterBox?.requestRender()
    responseSecondaryFooterBox?.requestRender()
    responseTertiaryFooterBox?.requestRender()
  }

  const promptMetaToneColor = (tone: PromptMetaTone) => theme[tone]

  const setPromptMetaRenderables = (parts: PromptMetaPart[]) => {
    if (parts.length === 0) {
      setTextRenderable(promptMetaProviderText, " ", theme.textMuted)
      setTextRenderable(promptMetaProviderDividerText, "", theme.textMuted)
      setTextRenderable(promptMetaModelText, "", theme.textMuted)
      setTextRenderable(promptMetaModelDividerText, "", theme.textMuted)
      setTextRenderable(promptMetaVariantText, "", theme.textMuted)
      setTextRenderable(promptMetaUsageDividerText, "", theme.textMuted)
      setTextRenderable(promptMetaUsageTokensText, "", theme.textMuted)
      setTextRenderable(promptMetaUsageBarOpenText, "", theme.textMuted)
      setTextRenderable(promptMetaUsageBarFilledText, "", theme.primary)
      setTextRenderable(promptMetaUsageBarEmptyText, "", theme.textMuted)
      setTextRenderable(promptMetaUsageBarCloseText, "", theme.textMuted)
      setTextRenderable(promptMetaUsagePercentText, "", theme.textMuted)
      return
    }

    const providerPart = parts[0]
    const modelPart = parts[1]
    const variantPart = parts[2]

    setTextRenderable(
      promptMetaProviderText,
      providerPart?.text ?? "",
      providerPart ? promptMetaToneColor(providerPart.tone) : theme.textMuted,
      providerPart ? TextAttributes.BOLD : TextAttributes.NONE,
    )
    setTextRenderable(promptMetaProviderDividerText, modelPart ? " • " : "", theme.textMuted)
    setTextRenderable(
      promptMetaModelText,
      modelPart?.text ?? "",
      modelPart ? promptMetaToneColor(modelPart.tone) : theme.textMuted,
      modelPart ? TextAttributes.BOLD : TextAttributes.NONE,
    )
    setTextRenderable(promptMetaModelDividerText, variantPart ? " • " : "", theme.textMuted)
    setTextRenderable(
      promptMetaVariantText,
      variantPart?.text ?? "",
      variantPart ? promptMetaToneColor(variantPart.tone) : theme.textMuted,
      variantPart ? TextAttributes.BOLD : TextAttributes.NONE,
    )

    const usage = promptUsageMeta()
    setTextRenderable(promptMetaUsageDividerText, usage ? " • " : "", theme.textMuted)
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

  const renderHistoryLoadingIndicator = () => {
    if (!historyLoadingBox) {
      return
    }
    historyLoadingBox.visible = centerMode() === "transcript" && loadingHistory()
    if (centerMode() !== "transcript") {
      if (historyLoadingText) {
        historyLoadingBox.remove(historyLoadingText.id)
        historyLoadingText.destroyRecursively()
        historyLoadingText = undefined
      }
    } else if (loadingHistory()) {
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

  const requestTranscriptRender = () => {
    if (uiBatchDepth > 0) {
      pendingTranscriptRender = true
      return
    }
    transcriptScrollbox?.requestRender()
  }

  const flushDeferredUiUpdates = () => {
    if (pendingTranscriptRender) {
      pendingTranscriptRender = false
      transcriptScrollbox?.requestRender()
    }
    if (pendingSessionChromeUpdate) {
      pendingSessionChromeUpdate = false
      updateSessionChrome()
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
      setTextRenderable(statusOpenText, "", theme.textMuted)
      for (const text of statusLabelTexts) {
        setTextRenderable(text, " ", theme.textMuted)
      }
      setTextRenderable(statusCloseText, "", theme.textMuted)
      statusIndicatorBox?.requestRender()
      return
    }
    const badge = focusedStatusBadge()
    setTextRenderable(statusOpenText, "", theme.textMuted)
    renderStatusBadgeTexts(statusLabelTexts, badge.label, badge.tone)
    setTextRenderable(statusCloseText, "", theme.textMuted)
    statusIndicatorBox?.requestRender()
  }

  const applyResponseLayout = () => {
    if (!responseLayoutBox || !responseTopRowBox || !responsePrimaryPane || !responseSecondaryPane || !responseTertiaryPane) {
      logViewDebug("apply response layout:missing refs", {
        has_layout_box: Boolean(responseLayoutBox),
        has_top_row_box: Boolean(responseTopRowBox),
        has_primary_pane: Boolean(responsePrimaryPane),
        has_secondary_pane: Boolean(responseSecondaryPane),
        has_tertiary_pane: Boolean(responseTertiaryPane),
      })
      return
    }

    const split = splitAgentResponseMode()
    const {
      primary: primaryAgent,
      secondary: secondaryAgent,
      tertiary: tertiaryAgent,
    } = responsePaneSelection()
    const primaryFocused = primaryAgent?.id === focusedAgentId()
    const secondaryFocused = secondaryAgent?.id === focusedAgentId()
    const tertiaryFocused = tertiaryAgent?.id === focusedAgentId()
    const geometry = computeSplitPaneGeometry(
      dimensions().width,
      split,
      Boolean(secondaryAgent),
      Boolean(tertiaryAgent),
    )
    const primarySurface = transcriptSurfacePalette(resolveTranscriptSurfaceTone(split, primaryFocused))
    const secondarySurface = transcriptSurfacePalette(resolveTranscriptSurfaceTone(split, secondaryFocused))
    const tertiarySurface = transcriptSurfacePalette(resolveTranscriptSurfaceTone(split, tertiaryFocused))
    const primaryBackground = split
      ? primarySurface.panel
      : theme.backgroundPanel
    const secondaryBackground = split
      ? secondarySurface.panel
      : theme.backgroundElement
    const tertiaryBackground = split
      ? tertiarySurface.panel
      : theme.backgroundElement
    const layoutSummary = applyResponseLayoutRenderables({
      renderables: {
        responseLayoutBox,
        responseTopRowBox,
        responsePrimaryPane,
        responseSecondaryPane,
        responseTertiaryPane,
        historyLoadingBox,
        transcriptScrollbox,
        responseSecondaryScrollbox,
        responseTertiaryScrollbox,
        responsePrimaryFooterBox,
        responseSecondaryFooterBox,
        responseTertiaryFooterBox,
      },
      geometry,
      split,
      primaryFocused,
      secondaryFocused,
      tertiaryFocused,
      primaryBackground,
      secondaryBackground,
      tertiaryBackground,
      primaryBorderColor: theme.primary,
      secondaryBorderColor: theme.primary,
      tertiaryBorderColor: theme.primary,
      subtleBorderColor: theme.borderSubtle,
    })

    renderSplitPaneFooters()

    syncAuxiliaryPane({
      scrollbox: responseSecondaryScrollbox,
      nextAgentId: secondaryAgent?.id ?? null,
      currentAgentId: responseSecondaryAgentId,
      splitMode: splitAgentResponseMode(),
      clearAuxiliaryAgentPane,
      unregisterAgentScrollbox: (agentId) => {
        agentTranscriptScrollboxes.delete(agentId)
      },
      assignCurrentAgentId: (value) => {
        responseSecondaryAgentId = value
      },
      registerAgentScrollbox: (agentId, scrollbox) => {
        agentTranscriptScrollboxes.set(agentId, scrollbox)
      },
      rebuildAuxiliaryAgentPane,
      buildEmptyTranscriptRenderable: () => buildEmptyTranscriptRenderable(renderer),
    })
    syncAuxiliaryPane({
      scrollbox: responseTertiaryScrollbox,
      nextAgentId: tertiaryAgent?.id ?? null,
      currentAgentId: responseTertiaryAgentId,
      splitMode: splitAgentResponseMode(),
      clearAuxiliaryAgentPane,
      unregisterAgentScrollbox: (agentId) => {
        agentTranscriptScrollboxes.delete(agentId)
      },
      assignCurrentAgentId: (value) => {
        responseTertiaryAgentId = value
      },
      registerAgentScrollbox: (agentId, scrollbox) => {
        agentTranscriptScrollboxes.set(agentId, scrollbox)
      },
      rebuildAuxiliaryAgentPane,
      buildEmptyTranscriptRenderable: () => buildEmptyTranscriptRenderable(renderer),
    })

    if (transcriptScrollbox) {
      rebuildTranscript()
    }
    if (secondaryAgent?.id) {
      rebuildAuxiliaryAgentPane(secondaryAgent.id)
    }
    if (tertiaryAgent?.id) {
      rebuildAuxiliaryAgentPane(tertiaryAgent.id)
    }

    scheduleResponsePaneRepaint()

    logViewDebug("apply response layout", {
      split,
      split_pane_width: layoutSummary.splitPaneWidth,
      secondary_visible: layoutSummary.secondaryVisible,
      tertiary_visible: layoutSummary.tertiaryVisible,
      primary_width: layoutSummary.primaryWidth,
      secondary_width: layoutSummary.secondaryWidth,
      secondary_agent_id: secondaryAgent?.id ?? null,
      tertiary_agent_id: tertiaryAgent?.id ?? null,
    })
  }

  createEffect(() => {
    splitAgentResponseMode()
    multiAgentResponseLayout()
    dimensions().width
    sessionState().agents.length
    focusedAgentId()
    providerRunState()?.model
    providerRunState()?.variant
    // Track session working state for busy badge updates
    working()
    activeStatusLabel()
    // Track agent states for busy badge updates in split view panes
    for (const agent of sessionState().agents) {
      agent.is_processing
      agent.state
    }
    agentActivityLabels()
    streamingAgentId()
    applyResponseLayout()
  })

  const rebuildSplitPaneTranscripts = () => {
    if (transcriptScrollbox) {
      rebuildTranscript()
    }
    for (const agentId of splitPaneAuxiliaryAgentIds(sessionState().agents, splitAgentResponseMode())) {
      rebuildAuxiliaryAgentPane(agentId)
    }
  }

  const refreshSplitPaneFocusRepaint = () => {
    if (!splitAgentResponseMode()) {
      return
    }

    const refreshToken = ++pendingSplitPaneRefresh
    const refresh = () => {
      if (refreshToken !== pendingSplitPaneRefresh || !splitAgentResponseMode()) {
        return
      }
      applyResponseLayout()
      rebuildSplitPaneTranscripts()
      scheduleResponsePaneRepaint()
    }

    refresh()
    startTimeout(refresh, 0)
    startTimeout(refresh, 16)
  }

  const updateSessionChrome = () => {
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
        previous.add(turnId)
      } else {
        previous.delete(turnId)
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

  const applyExpandedTurns = (entries: TranscriptEntry[], expandedTurnIds: readonly number[]) => {
    if (expandedTurnIds.length === 0) {
      return entries
    }

    let nextEntries = entries.map((entry) => ({ ...entry }))
    let changed = false
    for (const turnId of expandedTurnIds) {
      const expanded = expandTurnEntries(nextEntries, turnId)
      nextEntries = expanded.entries
      changed ||= expanded.changed
    }
    return changed ? nextEntries : entries
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
    if (splitAgentResponseMode()) {
      scheduleResponsePaneRepaint()
    }
  }

  const setAgentTranscriptEntries = (agentId: string, nextEntries: TranscriptEntry[]) => {
    const sanitizedEntries = nextEntries.filter(Boolean)
    setAgentPaneEntries((current) => ({
      ...current,
      [agentId]: sanitizedEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(sanitizedEntries))
    if (splitAgentResponseMode() && agentId === responsePrimaryAgent()?.id) {
      replaceTranscriptEntries(sanitizedEntries.map((entry) => ({ ...entry })))
    }
    if (splitAgentResponseMode() && agentId === responseSecondaryAgent()?.id) {
      rebuildAuxiliaryAgentPane(agentId)
    }
    if (splitAgentResponseMode() && agentId === responseTertiaryAgent()?.id) {
      rebuildAuxiliaryAgentPane(agentId)
    }
    if (splitAgentResponseMode()) {
      scheduleResponsePaneRepaint()
    }
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
    if (splitAgentResponseMode() && agentId === responsePrimaryAgent()?.id) {
      return entries.filter(Boolean).map((entry) => ({ ...entry }))
    }
    return (agentPaneEntries()[agentId] ?? []).map((entry) => ({ ...entry }))
  }

  const expandLatestPaneTurnForLiveUpdate = (items: TranscriptEntry[]) => {
    const nextItems = items.map((entry) => ({ ...entry }))
    return expandLatestTurnForLiveUpdateInPlace(nextItems) ? nextItems : items
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
    const toggleEntry = findVisibleTurnToggle(currentEntries, turnId, toggleEntryId)
    if (toggleEntryId !== undefined && !toggleEntry) {
      return
    }
    const expanding = toggleEntry?.toggleMode === "expand"
    const { entries: nextEntries, changed } = expanding
      ? expandTurnEntries(currentEntries, turnId)
      : collapseTurnEntries(currentEntries, turnId)

    if (!changed) {
      return
    }
    setExpandedTurnState(agentId, turnId, expanding)
    setAgentTranscriptEntries(agentId, nextEntries)
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
        surfaceTone,
      )
      renderables.set(entry.id, renderable)
      scrollbox.add(renderable.wrapper)
    }
    scrollbox.requestRender()
  }

  const registerAuxiliaryAgentPane = (agentId: string, value: ScrollBoxRenderable | undefined) => {
    if (!value) {
      agentTranscriptScrollboxes.delete(agentId)
      return
    }
    agentTranscriptScrollboxes.set(agentId, value)
    rebuildAuxiliaryAgentPane(agentId)
  }

  const pruneAuxiliaryAgentPanes = (session: RuntimeSession) => {
    const activeAgentIds = new Set(
      session.agents
        .map((agent) => agent.id)
        .filter((agentId) => agentId === session.agents[1]?.id || agentId === session.agents[2]?.id),
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

  const syncVisibleTranscriptPreview = () => {
    const agentId = visibleTranscriptAgentId()
    if (!agentId) {
      return
    }
    setAgentPanePreview(agentId, formatTranscriptPreview(entries.filter(Boolean)))
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

  const appendTranscriptEntryToAgentPane = (agentId: string, entry: Omit<TranscriptEntry, "id">) => {
    const currentEntries = expandLatestPaneTurnForLiveUpdate(currentAgentPaneEntries(agentId))
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
      trimAgentPaneEntries({
        entries: [...currentEntries, nextEntry],
        maxEntries: LIVE_TRANSCRIPT_LIMIT,
        maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
        onTrimmedMergeKey: (mergeKey) => {
          auxiliaryAgentPaneTools(agentId).delete(mergeKey)
        },
      }),
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

    const currentEntries = expandLatestPaneTurnForLiveUpdate(currentAgentPaneEntries(agentId))
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
        setAgentTranscriptEntries(agentId, trimAgentPaneEntries({
          entries: nextEntries,
          maxEntries: LIVE_TRANSCRIPT_LIMIT,
          maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
          onTrimmedMergeKey: (mergeKey) => {
            auxiliaryAgentPaneTools(agentId).delete(mergeKey)
          },
        }))
        return
      }
    }

    const last = nextEntries.at(-1)
    if (!mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
      last.text += normalized
      setAgentTranscriptEntries(agentId, trimAgentPaneEntries({
        entries: nextEntries,
        maxEntries: LIVE_TRANSCRIPT_LIMIT,
        maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
        onTrimmedMergeKey: (mergeKey) => {
          auxiliaryAgentPaneTools(agentId).delete(mergeKey)
        },
      }))
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
    setAgentTranscriptEntries(agentId, trimAgentPaneEntries({
      entries: nextEntries,
      maxEntries: LIVE_TRANSCRIPT_LIMIT,
      maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
      onTrimmedMergeKey: (mergeKey) => {
        auxiliaryAgentPaneTools(agentId).delete(mergeKey)
      },
    }))
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
    if (!agentId) {
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
      resolveVisibleAgentId: (agents, focusedAgentId) =>
        selectResponsePaneAgents(agents, focusedAgentId, splitAgentResponseMode()).visibleTranscriptAgentId,
      loadHistoryPage: async (agentId, cursor) => {
        const historyPage = await getSessionHistory(client, session.id, cursor, agentId)
        return {
          entries: historyPage.entries,
          nextCursor: historyPage.next_cursor,
        }
      },
      hydrateEntries: hydrateTranscriptEntries,
      stitchPrependedHistory,
      collapseHistoricalTurns,
      applyExpandedTurns,
      reindexEntries: reindexTranscriptEntries,
      formatPreview: formatTranscriptPreview,
    })

    pruneAuxiliaryAgentPanes(session)
    setExpandedTurnIdsByAgent(nextPaneState.expandedTurnIdsByAgent)
    setAgentPanePreviews(nextPaneState.previews)
    setAgentPaneEntries(nextPaneState.paneEntries)
    setNextHistoryCursor(nextPaneState.visibleCursor)
    replaceTranscriptEntries(
      (nextPaneState.visibleAgentId ? nextPaneState.paneEntries[nextPaneState.visibleAgentId] : nextPaneState.visibleEntries)
        ?.map((entry) => ({ ...entry })) ?? [],
    )
    if (splitAgentResponseMode()) {
      for (const agentId of splitPaneAuxiliaryAgentIds(session.agents, true)) {
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
      primaryTranscriptSurfaceTone(),
    )
    transcriptRenderables.set(entry.id, renderable)
    transcriptScrollbox.add(renderable.wrapper)
    if (requestRender) {
      requestTranscriptRender()
    }
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

  const measureVisibleTurnHeight = (turnId: number | null | undefined) => {
    if (!turnId) {
      return 0
    }
    return visibleTranscriptEntries()
      .filter((entry) => entry.turnId === turnId && !entry.historyDeferred)
      .reduce((total, entry) => total + (transcriptRenderables.get(entry.id)?.wrapper.height ?? 0), 0)
  }

  const restoreTurnScrollPosition = (
    scrollbox: ScrollBoxRenderable,
    turnId: number,
    previousScrollTop: number,
    previousViewportHeight: number,
    previousVisibleTurnHeight: number,
  ) => {
    const restoreToken = ++pendingHistoryScrollRestore
    const restoreScroll = (remainingAttempts: number, lastHeight = -1, stableFrames = 0) => {
      if (!transcriptScrollbox || transcriptScrollbox !== scrollbox || restoreToken !== pendingHistoryScrollRestore) {
        pendingHistoryScrollRestore = 0
        return
      }

      const nextVisibleTurnHeight = measureVisibleTurnHeight(turnId)
      const nextScrollTop = computeCollapsedHistoryScrollTop(
        previousScrollTop,
        previousVisibleTurnHeight,
        nextVisibleTurnHeight,
        previousViewportHeight,
      )

      scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: nextScrollTop })
      scrollbox.requestRender()
      lastTranscriptScrollTop = scrollbox.scrollTop

      const closeEnough = Math.abs(scrollbox.scrollTop - nextScrollTop) <= 1
      const nextStableFrames = nextVisibleTurnHeight === lastHeight ? stableFrames + 1 : 0
      if ((closeEnough && nextStableFrames >= 1) || remainingAttempts <= 1) {
        pendingHistoryScrollRestore = 0
        return
      }

      startTimeout(() => restoreScroll(remainingAttempts - 1, nextVisibleTurnHeight, nextStableFrames), 16)
    }

    scrollbox.requestRender()
    startTimeout(() => restoreScroll(4), 0)
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

    if (centerMode() === "tree" && directoryTreeState()) {
      const treeState = directoryTreeState()!
      const rows = buildDirectoryTreeRows(treeState)
      transcriptScrollbox.add(buildDirectoryTreeRenderable(renderer, treeState))
      const selectedIndex = Math.max(0, rows.findIndex((row) => row.id === treeState.selectedPath))
      const maxScrollTop = Math.max(0, transcriptScrollbox.scrollHeight - transcriptScrollbox.height)
      transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: Math.max(0, Math.min(selectedIndex, maxScrollTop)) })
      transcriptScrollbox.requestRender()
      return
    }

    const visibleEntries = visibleTranscriptEntries()
    if (visibleEntries.length === 0) {
      emptyTranscriptRenderable = isAttached()
        ? buildEmptyTranscriptRenderable(renderer)
        : buildNoSessionRenderable(renderer, waitingRoomState(), availableSessions(), providerCatalogState())
      transcriptScrollbox.add(emptyTranscriptRenderable)
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

  const replaceTranscriptEntries = (nextEntries: TranscriptEntry[]) => {
    const sanitizedEntries = nextEntries.filter(Boolean)
    tools.clear()
    currentTurnId = computeCurrentTurnId(sanitizedEntries)
    nextTurnId = computeNextTurnId(sanitizedEntries)
    setEntries(reconcile(sanitizedEntries))
    setEntryCounter(sanitizedEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    rebuildTranscript()
    lastTranscriptScrollTop = transcriptScrollbox?.scrollTop ?? 0
    syncVisibleTranscriptPreview()
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
    const nextCombinedEntries = collapseHistoricalTurns(
      stitchPrependedHistory(sanitizedEntries, currentEntries),
      sessionHasPromptWork(sessionState()),
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

  const transitionToNoSession = (message = "No session attached.") => {
    const nextDetachedState = deriveDetachedCliTransitionState({
      cliOptions: options,
      waitingRoomState: waitingRoomState(),
      message,
    })
    setAttachmentState(null)
    setProviderRunState(null)
    clearPendingPromptAttachments()
    setCenterMode(nextDetachedState.centerMode)
    setDirectoryTreeState(null)
    activeToolLabels.clear()
    setProviderActivityLabel(nextDetachedState.providerActivityLabel)
    setActiveStatusLabel(nextDetachedState.activeStatusLabel)
    setCreatedSessionState(nextDetachedState.createdSession)
    setSessionState(nextDetachedState.session)
    historyLoadGeneration += 1
    replaceTranscriptEntries([])
    setAgentPaneEntries(nextDetachedState.agentPaneEntries)
    setAgentPanePreviews(nextDetachedState.agentPanePreviews)
    setAgentActivityLabels(nextDetachedState.agentActivityLabels)
    setStreamingAgentId(nextDetachedState.streamingAgentId)
    agentTranscriptScrollboxes.clear()
    agentTranscriptRenderables.clear()
    agentEmptyTranscriptRenderables.clear()
    agentPaneTools.clear()
    setSubmitting(nextDetachedState.submitting)
    setWorking(nextDetachedState.working)
    stopRequestInFlight = false
    setFatalError(nextDetachedState.fatalError)
    setDaemonDisconnected(nextDetachedState.daemonDisconnected)
    setNextHistoryCursor(nextDetachedState.nextHistoryCursor)
    setHistoryLoadingState(false)
    setStatusLine(nextDetachedState.statusLine)
    updateSessionChrome()
    promptInput?.clear()
    syncPromptTextSnapshot()
    promptInput?.blur()
    reconcileWaitingRoom(nextDetachedState.waitingRoomState)
    void refreshWaitingRoomData()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }

  const detachCurrentAttachment = async () => {
    const attachment = attachmentState()
    if (!attachment) {
      return
    }
    await client.send(detachFromSessionRequest(attachment.id))
    setAttachmentState(null)
  }

  const attachBinding = async (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
    launch: { model: string; effort: string } = { model: options.model, effort: options.effort },
  ) => {
    const currentAttachment = attachmentState()
    if (currentAttachment?.session_id === session.id) {
      return
    }
    if (currentAttachment) {
      await detachCurrentAttachment()
    }
    clearPendingPromptAttachments()
    historyLoadGeneration += 1
    const attachment = await attachToSession(client, session.id, options.clientId)
    const attachedSession = await getSessionState(client, session.id)
    if (!attachedSession.active_provider_run_id) {
      options.model = launch.model
      options.effort = launch.effort
      const run = await launchProviderRun(client, session.id, options.accountProfile, launch.model, launch.effort, attachedSession.focused_agent_id)
      logProviderRunDebug("attached session launched provider run", run, {
        session_id: session.id,
        requested_model: launch.model,
        requested_variant: launch.effort,
      })
      setProviderRunState(run)
    } else {
      const run = await tryGetProviderRun(client, attachedSession.active_provider_run_id, appLogger)
      logProviderRunDebug("attached session loaded existing provider run", run, {
        session_id: session.id,
        requested_model: options.model,
      })
      setProviderRunState(run)
    }
    setProviderCatalogState(await getProviderCatalog(client, appLogger))
    reconcileWaitingRoom(waitingRoomState())
    await maybeResize(client, session.id)
    await catchUpAttachedSession(client, session.id, attachment.id, attachedSession, appLogger)
    const hydratedSession = await getSessionState(client, session.id)
    const nextAttachedState = deriveAttachedCliTransitionState({
      session: hydratedSession,
      createdSession,
      connectedStatus: DEFAULT_CONNECTED_STATUS,
    })
    setAttachmentState(attachment)
    setCreatedSessionState(nextAttachedState.createdSession)
    setSessionState(nextAttachedState.session)
    setCenterMode(nextAttachedState.centerMode)
    setDirectoryTreeState(null)
    activeToolLabels.clear()
    setProviderActivityLabel(nextAttachedState.providerActivityLabel)
    setActiveStatusLabel(nextAttachedState.activeStatusLabel)
    await refreshAgentPanes(hydratedSession)
    setFatalError(nextAttachedState.fatalError)
    setDaemonDisconnected(nextAttachedState.daemonDisconnected)
    setSubmitting(nextAttachedState.submitting)
    setWorking(nextAttachedState.working)
    setStatusLine(nextAttachedState.statusLine)
    updateSessionChrome()
    promptInput?.focus()
    setAvailableSessions(await listSessions(client))
    scheduleShortViewportHistoryCheck()
  }

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

  const handleSessionCommand = async (commandLine: string) => {
    const [_, action, ...args] = commandLine.trim().split(/\s+/)
    const value = args.join(" ").trim()

    switch (action) {
      case "create":
      case "new": {
        const session = await createSession(client, options.workspace ?? process.cwd(), options.worktree ?? options.workspace ?? process.cwd(), value || undefined)
        await attachBinding(session, true)
        flashFooter(`attached to session ${session.alias ?? session.id}`, "info")
        return true
      }
      case "attach": {
        if (!value) {
          flashFooter("usage: /session attach <ref>", "error")
          return true
        }
        const session = await resolveSession(client, value, options.workspace ?? process.cwd())
        await attachBinding(session, false)
        flashFooter(`attached to session ${session.alias ?? session.id}`, "info")
        return true
      }
      case "list":
      case "ls": {
        const sessions = await listSessions(client)
        appendNotice(formatSessionList(sessions, sessionState().id))
        flashFooter(`listed ${sessions.length} session${sessions.length === 1 ? "" : "s"}`, "info")
        return true
      }
      case "delete": {
        const sessionRef = value || (isAttached() ? sessionState().id : "")
        if (!sessionRef) {
          flashFooter("usage: /session delete <ref>", "error")
          return true
        }
        const deleted = await deleteSessionByRef(client, sessionRef, options.workspace ?? process.cwd())
        if (isAttached() && deleted.id === sessionState().id) {
          transitionToNoSession(`Session ${deleted.alias ?? deleted.id} was deleted.`)
        } else {
          flashFooter(`deleted session ${deleted.alias ?? deleted.id}`, "info")
        }
        return true
      }
      default:
        return false
    }
  }

  const handleProviderCommand = async (commandLine: string) => {
    const value = commandLine.replace(/^\/provider\s*/, "").trim()
    if (!value) {
      flashFooter("usage: /provider opencode", "error")
      return
    }
    if (value !== "opencode") {
      flashFooter(`unknown provider: ${value}`, "error")
      return
    }
    flashFooter("OpenCode selected", "info")
  }

  const handleModelCommand = async (commandLine: string) => {
    const value = commandLine.replace(/^\/model\s*/, "").trim()
    if (!value) {
      flashFooter("usage: /model <provider/model>", "error")
      return
    }
    await applyModelSelection(value)
  }

  const handleVariantCommand = async (commandLine: string) => {
    const value = commandLine.replace(/^\/variant\s*/, "").trim()
    if (!value) {
      flashFooter("usage: /variant <name>", "error")
      return
    }
    await applyVariantSelection(value)
  }

  const handleViewCommand = async (commandLine: string) => {
    const value = commandLine.replace(/^\/view\s*/, "").trim().toLowerCase()
    if (!value) {
      flashFooter(
        `view: ${multiAgentResponseLayout()} • agents: ${sessionState().agents.length}`,
        "info",
      )
      return
    }
    if (value !== "split" && value !== "individual") {
      flashFooter("usage: /view <split|individual>", "error")
      return
    }
    const nextLayout = value as MultiAgentResponseLayout
    appLogger?.info("handling view command", {
      requested_layout: nextLayout,
      previous_layout: multiAgentResponseLayout(),
      attached: isAttached(),
      agent_count: sessionState().agents.length,
      focused_agent_id: focusedAgentId(),
    })
    setMultiAgentResponseLayout(nextLayout)
    logViewDebug("view command:after set layout", {
      requested_layout: nextLayout,
    })
    applyResponseLayout()
    if (isAttached() && attachmentState()) {
      const updated = await updateSessionConfig(
        client,
        sessionState().id,
        attachmentState()!.id,
        { [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: nextLayout },
        false,
      )
      applySessionState(updated.session)
      await refreshAgentPanes(updated.session)
    }
    await saveUiPreferences({ multiAgentResponseLayout: nextLayout })
    rebuildTranscript()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
    startTimeout(() => {
      logViewDebug("view command:post render tick", {
        requested_layout: nextLayout,
        current_focus: describeRenderableDebug((renderer as { currentFocusedRenderable?: Renderable | null }).currentFocusedRenderable ?? null),
      })
    }, 0)
    flashFooter(`view set to ${nextLayout} • ${sessionState().agents.length} agents`, "info")
  }

  const handleCycleAgentFocus = async () => {
    if (!isAttached()) {
      flashFooter("must be attached to a session to cycle agents", "error")
      return
    }
    try {
      const response = await client.send<Record<string, unknown>>(
        cycleAgentFocusRequest(sessionState().id),
      )
      const payload = expectVariant<{ agent: AgentInstance | null }>(response, "AgentFocusCycled")
      const nextSession = await getSessionState(client, sessionState().id)
      const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(nextSession)
      applySessionState(nextSession)
      if (shouldRefreshPanes) {
        await refreshAgentPanes(nextSession)
      }
      if (!nextSession.active_provider_run_id && payload.agent) {
        const run = await launchProviderRun(
          client,
          sessionState().id,
          options.accountProfile,
          payload.agent.model ?? currentModelId(),
          currentVariantId(),
          payload.agent.id,
        )
        setProviderRunState(run)
        applySessionState(await getSessionState(client, sessionState().id))
      }
      if (payload.agent) {
        flashFooter(`cycled to agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`, "info")
      } else {
        flashFooter("no agents to cycle", "info")
      }
    } catch (error) {
      flashFooter(formatError(error), "error")
    }
  }

  const handleAgentCommand = async (commandLine: string) => {
    const args = commandLine.replace(/^\/agent\s*/, "").trim().split(/\s+/)
    const subcommand = args[0]

    if (!isAttached()) {
      flashFooter("must be attached to a session to manage agents", "error")
      return
    }

    switch (subcommand) {
      case "spawn": {
        const alias = args[1]
        const model = args[2]
        const provider = providerRunState()?.provider ?? "opencode"
        try {
          const response = await client.send<Record<string, unknown>>(
            spawnAgentRequest(sessionState().id, provider, alias, model),
          )
          const payload = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned")
          const run = await launchProviderRun(
            client,
            sessionState().id,
            options.accountProfile,
            model ?? currentModelId(),
            currentVariantId(),
            payload.agent.id,
          )
          setProviderRunState(run)
          const nextSession = await getSessionState(client, sessionState().id)
          const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(nextSession)
          applySessionState(nextSession)
          if (shouldRefreshPanes) {
            await refreshAgentPanes(nextSession)
          }
          rebuildTranscript()
          flashFooter(`spawned agent ${payload.agent.agent_ref}${alias ? ` (${alias})` : ""}`, "info")
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
        return
      }
      case "delete":
      case "destroy": {
        const reference = args[1]
        const resolved = resolveSessionAgent(reference)
        if (resolved.error || !resolved.agent) {
          flashFooter(resolved.error ?? "usage: /agent delete <agent-name|agent-alias>", "error")
          return
        }
        try {
          await client.send<Record<string, unknown>>(
            destroyAgentRequest(sessionState().id, resolved.agent.id),
          )
          const nextSession = await getSessionState(client, sessionState().id)
          const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(nextSession)
          applySessionState(nextSession)
          if (shouldRefreshPanes) {
            await refreshAgentPanes(nextSession)
          }
          rebuildTranscript()
          refreshSplitPaneFocusRepaint()
          flashFooter(`deleted agent ${formatAgentLabel(resolved.agent)}`, "info")
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
        return
      }
      case "focus": {
        const agentId = args[1]
        if (!agentId) {
          flashFooter("usage: /agent focus <agent-id>", "error")
          return
        }
        try {
          const response = await client.send<Record<string, unknown>>(
            focusAgentRequest(sessionState().id, agentId),
          )
          const payload = expectVariant<{ agent: AgentInstance }>(response, "AgentFocused")
          const nextSession = await getSessionState(client, sessionState().id)
          const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(nextSession)
          applySessionState(nextSession)
          if (shouldRefreshPanes) {
            await refreshAgentPanes(nextSession)
          }
          if (!nextSession.active_provider_run_id) {
            const run = await launchProviderRun(
              client,
              sessionState().id,
              options.accountProfile,
              payload.agent.model ?? currentModelId(),
              currentVariantId(),
              payload.agent.id,
            )
            setProviderRunState(run)
            applySessionState(await getSessionState(client, sessionState().id))
          }
          flashFooter(`focused on agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`, "info")
        } catch (error) {
          flashFooter(formatError(error), "error")
        }
        return
      }
      case "list":
      case "ls": {
        const agents = sessionState().agents
        if (agents.length === 0) {
          flashFooter("no agents in session", "info")
        } else {
          const agentList = agents.map(a => `${a.agent_ref}${a.alias ? ` (${a.alias})` : ""} [${a.state}]`).join(", ")
          flashFooter(`${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`, "info")
        }
        return
      }
      case "cycle": {
        await handleCycleAgentFocus()
        return
      }
      default:
        flashFooter("usage: /agent spawn [alias] [model] | delete [agent-name|agent-alias] | focus <agent-id> | list | cycle", "error")
    }
  }

  const recoverProviderRun = async (reason: string) => {
    if (!isAttached() || providerRecoveryInFlight) {
      return
    }
    providerRecoveryInFlight = true
    try {
      const run = await launchProviderRun(
        client,
        sessionState().id,
        options.accountProfile,
        currentModelId(),
        currentVariantId(),
        focusedAgentId(),
      )
      setProviderRunState(run)
      applySessionState(await getSessionState(client, sessionState().id))
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
    if (value === "/exit") {
      await requestExit()
      return
    }
    if (value === "/waiting") {
      await requestWaitingRoom()
      return
    }
    if (value === "/stop") {
      await requestPromptStop()
      return
    }
    if (value === "/session list") {
      await handleSessionCommand(value)
      return
    }
    if (value.startsWith("/provider ")) {
      await handleProviderCommand(value)
      return
    }
    if (value.startsWith("/model ")) {
      await handleModelCommand(value)
      return
    }
    if (value.startsWith("/variant ")) {
      await handleVariantCommand(value)
      return
    }
    if (value.startsWith("/view")) {
      await handleViewCommand(value)
      return
    }
    if (value.startsWith("/agent ")) {
      try {
        await handleAgentCommand(value)
      } catch (error) {
        flashFooter(formatError(error), "error")
      }
      return
    }
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
    if (trimmed === "/exit") {
      await requestExit()
      return
    }
    if (trimmed === "/waiting") {
      await requestWaitingRoom()
      return
    }
    if (trimmed.startsWith("/attach")) {
      try {
        await handleAttachmentCommand(rawPrompt)
      } catch (error) {
        appLogger?.error("attachment command failed", {
          command: trimmed,
          error: formatError(error),
        })
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
      }
      return
    }
    if (trimmed.startsWith("/session")) {
      try {
        const handled = await handleSessionCommand(trimmed)
        if (!handled) {
          flashFooter("unknown /session command", "error")
        }
      } catch (error) {
        appLogger?.error("session command failed", {
          command: trimmed,
          error: formatError(error),
        })
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
      }
      return
    }
    if (trimmed.startsWith("/provider")) {
      try {
        await handleProviderCommand(trimmed)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
        clearCommandCenter()
      }
      return
    }
    if (trimmed.startsWith("/model")) {
      try {
        await handleModelCommand(trimmed)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
        clearCommandCenter()
      }
      return
    }
    if (trimmed.startsWith("/variant")) {
      try {
        await handleVariantCommand(trimmed)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
        clearCommandCenter()
      }
      return
    }
    if (trimmed.startsWith("/agent")) {
      try {
        await handleAgentCommand(trimmed)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
        clearCommandCenter()
      }
      return
    }
    if (trimmed.startsWith("/view")) {
      try {
        await handleViewCommand(trimmed)
      } catch (error) {
        flashFooter(formatError(error), "error")
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
        clearCommandCenter()
      }
      return
    }
    if (trimmed === "/stop") {
      try {
        await requestPromptStop()
      } finally {
        promptInput.clear()
        syncPromptTextSnapshot()
      }
      return
    }
    if (!isAttached()) {
      flashFooter(SESSION_NEW_ERROR_HINT, "error")
      promptInput.clear()
      syncPromptTextSnapshot()
      return
    }

    const prompt = trimmed ? (rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`) : ""
    const attachments = pendingAttachments().map<PromptAttachmentPart>((file) => ({
      url: file.url,
      mime: file.mime,
      filename: file.filename,
    }))
    const targetAgentId = focusedAgentId()
    try {
      appLogger?.info("submitting prompt", {
        chars: prompt.length,
        attachments: attachments.length,
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
      const response = await submitPromptWithRecovery(
        client,
        sessionState().id,
        attachment.id,
        prompt,
        attachments,
        options,
        appLogger,
      )
      const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
      applySessionState(payload.session)
      setStreamingAgentId(payload.session.active_prompt?.target_agent_id ?? targetAgentId)
      appendUserPrompt(renderPromptTranscript(prompt), payload.session.active_prompt?.target_agent_id ?? targetAgentId)
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
      promptInput.clear()
      syncPromptTextSnapshot()
      clearPendingPromptAttachments()
    } catch (error) {
      appLogger?.error("prompt submission failed", {
        error: formatError(error),
      })
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
    if (hotkeysOpen()) {
      event.preventDefault()
      event.stopPropagation()
    }
  })

  const handleSigint = () => {
    void (activePrompt() ? requestPromptStop() : requestExit())
  }
  const shouldNavigatePromptTurns = (event: { name: string; eventType: string; shift?: boolean }) => {
    if (!isAttached() || centerMode() !== "transcript" || event.eventType === "release") {
      return false
    }
    if (event.name !== "up" && event.name !== "down") {
      return false
    }
    return Boolean(event.shift) || !(promptInput?.plainText.trim())
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
    if (event.eventType !== "release" && hotkeysOpen() && event.name === "escape") {
      closeHotkeys()
      return
    }
    if (event.eventType !== "release" && event.ctrl && event.name === "e") {
      void requestExit()
      return
    }
    if (promptInput?.focused && commandCenterOpen()) {
      if (event.eventType !== "release" && event.name === "escape") {
        clearCommandCenter()
      }
      return
    }
    if (event.eventType !== "release" && event.name === "tab") {
      if (hotkeysOpen()) {
        return
      }
      void toggleCenterMode()
      return
    }
    if (event?.ctrl && event.name === "c") {
      void (activePrompt() ? requestPromptStop() : requestExit())
      return
    }
    if (event.eventType !== "release" && event.ctrl && event.name === "a") {
      void handleCycleAgentFocus()
      return
    }
    if (hotkeysOpen()) {
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
    if (isAttached() && centerMode() === "tree") {
      if (event.eventType !== "release" && event.name === "up") {
        navigateDirectoryTree("up")
        return
      }
      if (event.eventType !== "release" && event.name === "down") {
        navigateDirectoryTree("down")
        return
      }
      if (event.eventType !== "release" && (event.name === "return" || event.name === "enter") && !(promptInput?.plainText.trim())) {
        activateDirectoryTreeSelection()
        return
      }
    }
    if (shouldNavigatePromptTurns(event)) {
      navigatePromptTurns(event.name === "up" ? "previous" : "next")
      return
    }
    if (!isAttached()) {
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
              ? moveWaitingRoomFocus(next, -1)
              : keyName === "down"
                ? moveWaitingRoomFocus(next, 1)
                : cycleWaitingRoomValue(next, availableSessions(), providerCatalogState(), keyName === "left" ? -1 : 1),
          )
          return
        }
        setWaitingRoomState(next)
        rebuildTranscript()
        return
      }
      if (event.eventType !== "release" && (event.name === "return" || event.name === "enter")) {
        void activateWaitingRoom()
      }
    }
  }
  process.on("SIGINT", handleSigint)
  process.stdin.on("data", handleStdinData)
  onCleanup(() => {
    process.off("SIGINT", handleSigint)
    process.stdin.off("data", handleStdinData)
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
    if (!scrollbox) {
      return
    }
    if (pendingHistoryScrollRestore > 0) {
      return
    }

    const currentScrollTop = scrollbox.scrollTop
    if (currentScrollTop === 0 && lastTranscriptScrollTop > 0 && nextHistoryCursor() !== null && !loadingHistory()) {
      void loadOlderHistoryPage()
    }
    lastTranscriptScrollTop = currentScrollTop
  }

  const maybeLoadOlderHistoryForShortViewport = () => {
    const scrollbox = transcriptScrollbox
    if (!scrollbox || !isAttached() || loadingHistory() || nextHistoryCursor() === null) {
      return
    }
    if (scrollbox.scrollTop === 0 && scrollbox.scrollHeight <= scrollbox.height) {
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

  // Check if connection appears stale (working but no data received)
  const checkConnectionHealth = () => {
    if (!isAttached() || !working()) {
      consecutiveSilentPolls = 0
      return
    }

    const timeSinceLastActivity = Date.now() - lastDaemonActivityAt
    const isSilent = timeSinceLastActivity > 2000 // 2 seconds without activity

    if (isSilent) {
      consecutiveSilentPolls++
    } else {
      consecutiveSilentPolls = 0
    }

    // If we've had too many silent polls while "working", something may be wrong
    if (consecutiveSilentPolls >= SILENT_POLL_THRESHOLD) {
      appLogger?.warn("connection appears stale - no activity while working", {
        consecutive_silent_polls: consecutiveSilentPolls,
        time_since_last_activity_ms: timeSinceLastActivity,
      })
      // Trigger recovery
      void recoverProviderRun("stale connection - no activity received")
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

  const runPollingLoop = async (
    operation: string,
    intervalMs: number,
    task: () => Promise<void>,
  ) => {
    let consecutiveFailures = 0

    while (!closing) {
      try {
        await task()
        markPollerRecovered(operation, consecutiveFailures)
        consecutiveFailures = 0
      } catch (error) {
        if (closing) {
          break
        }
        if (isSessionUnavailableError(error)) {
          appLogger?.info("session became unavailable; returning to unattached state", {
            operation,
            error: formatError(error),
          })
          transitionToNoSession("Current session is no longer available.")
          consecutiveFailures = 0
          continue
        }
        consecutiveFailures += 1
        appLogger?.warn("poll operation failed", {
          operation,
          error: formatError(error),
          attempt: consecutiveFailures,
        })
        const decision = getPollRecoveryDecision(operation, error, consecutiveFailures)
        if (decision.retry) {
          markPollerDegraded(operation, decision.message)
          await sleep(decision.delayMs)
          continue
        }
        appLogger?.error("poll operation became fatal", {
          operation,
          error: formatError(error),
        })
        if (error instanceof Error && /local transport/i.test(error.message)) {
          setDaemonDisconnected(true)
        }
        setFatalError(formatError(error))
        updateSessionChrome()
        break
      }
      await sleep(intervalMs)
    }
  }

  const pollOutput = async () => {
    await runPollingLoop("polling terminal output", 50, async () => {
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
          void recoverProviderRun("terminal polling")
          return
        }
        throw error
      }
      const payload = expectVariant<{ records: TerminalOutputRecord[] }>(response, "TerminalOutput")
      if (payload.records.length > 0) {
        recordDaemonActivity("terminal_output")
      }
      queueTerminalOutputRecords(payload.records)
    })
  }

  const pollNotices = async () => {
    await runPollingLoop("polling runtime notices", 150, async () => {
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
    })
  }

  const pollSessionState = async () => {
    await runPollingLoop("polling session state", 250, async () => {
      if (!isAttached()) {
        return
      }
      const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionState().id))
      recordDaemonActivity("session_state_poll")
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
      const shouldRefreshPanes = shouldRefreshAgentPanesForSessionChange(payload.session)
      applySessionState(payload.session)
      if (shouldRefreshPanes) {
        await refreshAgentPanes(payload.session)
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
          updateSessionChrome()
        }
      } else if (providerRunState()) {
        logProviderRunDebug("session poll cleared provider run", providerRunState(), {
          session_id: payload.session.id,
        })
        setProviderRunState(null)
        updateSessionChrome()
        if (!sessionHasPromptWork(payload.session)) {
          void recoverProviderRun("missing active provider run")
        }
      }
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
    appLogger?.info("starting background pollers")
    void pollOutput()
    void pollNotices()
    void pollSessionState()
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
    if (pendingTurnCompletion) {
      clearTimeout(pendingTurnCompletion)
    }
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
    if (isAttached()) {
      return
    }
    const state = waitingRoomState()
    if (state.introStep >= 12) {
      return
    }
    setWaitingRoomState({
      ...state,
      introStep: state.introStep + 1,
    })
    rebuildTranscript()
  }, 90)

  onCleanup(() => {
    clearInterval(waitingRoomAnimation)
  })

  onMount(() => {
    if (isAttached()) {
      void refreshAgentPanes(sessionState())
    }
  })

  return (
    <box
      width={dimensions().width}
      height={dimensions().height}
      flexDirection="column"
      paddingBottom={1}
      paddingLeft={2}
      paddingRight={2}
      backgroundColor={theme.background}
    >
      <box
        flexGrow={1}
        backgroundColor={theme.backgroundPanel}
        border={["left", "right"]}
        customBorderChars={SplitBorder.customBorderChars}
        borderColor={theme.borderSubtle}
        onMouseUp={(event) => {
          if (event.button !== MouseButton.LEFT) {
            return
          }
          startTimeout(() => {
            copySelection()
          }, 0)
        }}
      >
        <box
          ref={(value) => {
            responseLayoutBox = value
            logViewDebug("mounted response layout box")
            applyResponseLayout()
          }}
          flexGrow={1}
          flexDirection="row"
          gap={0}
          paddingLeft={1}
          paddingRight={1}
          paddingTop={1}
          paddingBottom={1}
        >
          <box
            flexGrow={1}
            flexDirection="row"
            gap={0}
            ref={(value) => {
              responseTopRowBox = value
              applyResponseLayout()
            }}
          >
            <box
              ref={(value) => {
                responsePrimaryPane = value
                logViewDebug("mounted response primary pane")
                applyResponseLayout()
              }}
              flexGrow={1}
              flexDirection="column"
              border={["left"]}
              borderColor={theme.borderSubtle}
              backgroundColor={theme.backgroundPanel}
            >
              <box
                ref={(value) => {
                  historyLoadingBox = value
                  logViewDebug("mounted history loading box")
                  renderHistoryLoadingIndicator()
                }}
                flexShrink={0}
                paddingLeft={1}
                paddingRight={1}
              />
              <scrollbox
                ref={(value) => {
                  transcriptScrollbox = value
                  logViewDebug("mounted primary transcript scrollbox")
                  rebuildTranscript()
                  ensureBackgroundPollersStarted()
                }}
                flexGrow={1}
                stickyScroll={true}
                stickyStart="bottom"
                paddingLeft={2}
                paddingRight={1}
                paddingTop={1}
                paddingBottom={1}
                viewportOptions={{
                  paddingRight: 1,
                }}
                verticalScrollbarOptions={{
                  visible: true,
                  paddingLeft: 1,
                  trackOptions: {
                    backgroundColor: theme.backgroundElement,
                    foregroundColor: theme.border,
                  },
                }}
              />
              <box
                ref={(value) => {
                  responsePrimaryFooterBox = value
                  renderSplitPaneFooters()
                  applyResponseLayout()
                }}
                flexShrink={0}
                flexDirection="row"
                gap={1}
                paddingLeft={1}
                paddingRight={1}
              />
            </box>
            <box
              ref={(value) => {
                responseSecondaryPane = value
                logViewDebug("mounted response secondary pane")
                applyResponseLayout()
              }}
              width={0}
              flexShrink={0}
              flexDirection="column"
              border={false}
              borderColor={theme.borderSubtle}
              backgroundColor={theme.backgroundElement}
              paddingLeft={0}
              paddingRight={0}
              paddingTop={0}
              paddingBottom={0}
              visible={false}
            >
              <scrollbox
                ref={(value) => {
                  responseSecondaryScrollbox = value
                  logViewDebug("mounted response secondary scrollbox")
                  applyResponseLayout()
                }}
                flexGrow={1}
                stickyScroll={true}
                stickyStart="bottom"
                paddingLeft={2}
                paddingRight={1}
                paddingTop={1}
                paddingBottom={1}
                viewportOptions={{
                  paddingRight: 1,
                }}
                verticalScrollbarOptions={{
                  visible: true,
                  paddingLeft: 1,
                  trackOptions: {
                    backgroundColor: theme.backgroundElement,
                    foregroundColor: theme.border,
                  },
                }}
              />
              <box
                ref={(value) => {
                  responseSecondaryFooterBox = value
                  renderSplitPaneFooters()
                  applyResponseLayout()
                }}
                flexShrink={0}
                flexDirection="row"
                gap={1}
                paddingLeft={1}
                paddingRight={1}
              />
            </box>
          </box>
          <box
            ref={(value) => {
              responseTertiaryPane = value
              logViewDebug("mounted response tertiary pane")
              applyResponseLayout()
            }}
            width={0}
            flexShrink={0}
            flexDirection="column"
            border={false}
            borderColor={theme.borderSubtle}
            backgroundColor={theme.backgroundElement}
            paddingLeft={0}
            paddingRight={0}
            paddingTop={0}
            paddingBottom={0}
            visible={false}
          >
            <scrollbox
              ref={(value) => {
                responseTertiaryScrollbox = value
                logViewDebug("mounted response tertiary scrollbox")
                applyResponseLayout()
              }}
              flexGrow={1}
              stickyScroll={true}
              stickyStart="bottom"
              paddingLeft={2}
              paddingRight={1}
              paddingTop={1}
              paddingBottom={1}
              viewportOptions={{
                paddingRight: 1,
              }}
              verticalScrollbarOptions={{
                visible: true,
                paddingLeft: 1,
                trackOptions: {
                  backgroundColor: theme.backgroundElement,
                  foregroundColor: theme.border,
                },
              }}
            />
            <box
              ref={(value) => {
                responseTertiaryFooterBox = value
                renderSplitPaneFooters()
                applyResponseLayout()
              }}
              flexShrink={0}
              flexDirection="row"
              gap={1}
              paddingLeft={1}
              paddingRight={1}
            />
          </box>
        </box>
      </box>

      <box
        flexShrink={0}
        marginTop={1}
        overflow="visible"
        border={["left"]}
        borderColor={fatalError() ? theme.error : theme.primary}
        customBorderChars={PromptBorderChars}
      >
        <box
          overflow="visible"
          paddingLeft={2}
          paddingRight={2}
          paddingTop={1}
          paddingBottom={1}
          backgroundColor={theme.backgroundElement}
          flexDirection="column"
          gap={1}
        >
          <box
            ref={(value) => {
              commandCenterBox = value
              renderCommandCenter()
            }}
            position="absolute"
            left={0}
            right={0}
            flexDirection="column"
            overflow="visible"
          />
          {isAttached()
            ? (
                <textarea
                  ref={(value) => {
                    promptInput = value
                    value.syntaxStyle = promptTokenStyle
                    syncPromptPlaceholder()
                    syncPromptTextSnapshot()
                    refreshPromptAttachmentHighlights()
                    ensureBackgroundPollersStarted()
                  }}
                  placeholder={ATTACHED_PROMPT_PLACEHOLDER}
                  textColor={theme.text}
                  focusedTextColor={theme.text}
                  minHeight={1}
                  maxHeight={6}
                  keyBindings={PROMPT_KEYBINDINGS}
                  onKeyDown={(event) => {
                    if (handleCommandCenterKey(event)) {
                      return
                    }
                    handleHotkeysToggleShortcut("textarea", event)
                  }}
                  onContentChange={() => {
                    handlePromptContentChange()
                  }}
                  onSubmit={() => {
                    if (commandCenterOpen()) {
                      if (selectCommandCenterFromSubmit()) {
                        return
                      }
                    }
                    void submitPrompt()
                  }}
                />
              )
            : (
                <textarea
                  ref={(value) => {
                    promptInput = value
                    value.syntaxStyle = promptTokenStyle
                    syncPromptPlaceholder()
                    syncPromptTextSnapshot()
                    refreshPromptAttachmentHighlights()
                    ensureBackgroundPollersStarted()
                  }}
                  placeholder={SESSION_NEW_PLACEHOLDER}
                  textColor={theme.text}
                  focusedTextColor={theme.text}
                  minHeight={1}
                  maxHeight={6}
                  keyBindings={PROMPT_KEYBINDINGS}
                  onKeyDown={(event) => {
                    if (handleCommandCenterKey(event)) {
                      return
                    }
                    handleHotkeysToggleShortcut("textarea", event)
                  }}
                  onContentChange={() => {
                    handlePromptContentChange()
                  }}
                  onSubmit={() => {
                    if (commandCenterOpen()) {
                      if (selectCommandCenterFromSubmit()) {
                        return
                      }
                    }
                    void submitPrompt()
                  }}
                />
              )}
          <box flexDirection="row">
            <text
              ref={(value) => {
                promptMetaProviderText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {" "}
            </text>
            <text
              ref={(value) => {
                promptMetaProviderDividerText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaModelText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaModelDividerText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaVariantText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsageDividerText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsageTokensText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsageBarOpenText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsageBarFilledText = value
                updateSessionChrome()
              }}
              fg={theme.primary}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsageBarEmptyText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsageBarCloseText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
            <text
              ref={(value) => {
                promptMetaUsagePercentText = value
                updateSessionChrome()
              }}
              fg={theme.textMuted}
            >
              {""}
            </text>
          </box>
        </box>
      </box>

      <box flexShrink={0} marginTop={1} paddingLeft={2} paddingRight={2}>
        <box flexDirection="row" gap={1}>
          <box
            ref={(value) => {
              statusIndicatorBox = value
              updateSessionChrome()
            }}
            flexDirection="row"
          />
          <box
            ref={(value) => {
              footerSummaryBox = value
              updateSessionChrome()
            }}
            flexDirection="row"
          />
        </box>
      </box>

      {hotkeysOpen()
        ? null
        : null}
      <box
        ref={(value) => {
          hotkeysOverlayBox = value
          renderHotkeysOverlay()
        }}
        position="absolute"
        left={0}
        top={0}
      />
    </box>
  )
}

function hydrateTranscriptEntries(historyEntries: SessionHistoryPageEntry[]): TranscriptEntry[] {
  const mergedHistoryEntries = mergeAdjacentHistoryPageEntries(historyEntries)
  const entries: TranscriptEntry[] = []
  const tools = new Map<string, ToolTranscriptUpdate>()
  let nextId = 0
  let currentTurnId = 0

  const appendTranscriptEntry = (
    role: TranscriptEntry["role"],
    chunk: string,
    options: {
      mergeKey?: string
      sourceText?: string
      emphasis?: TranscriptEntry["emphasis"]
      historyEntryIndex?: number
      historyFragmentStart?: number
      historyFragmentEnd?: number
      historyTotalChars?: number
      turnId?: number
    } = {},
  ) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }

    if (options.mergeKey) {
      for (let index = entries.length - 1; index >= 0; index -= 1) {
        const candidate = entries[index]
        if (candidate?.role === role && candidate.mergeKey === options.mergeKey) {
          if (role === "assistant" || role === "reasoning") {
            candidate.text += normalized
            if (options.sourceText !== undefined) {
              candidate.sourceText = `${candidate.sourceText ?? ""}${options.sourceText}`
            }
          } else {
            candidate.text = normalized
            if (options.sourceText !== undefined) candidate.sourceText = options.sourceText
          }
          if (options.emphasis !== undefined) candidate.emphasis = options.emphasis
          if (options.historyEntryIndex !== undefined) candidate.historyEntryIndex = options.historyEntryIndex
          if (options.historyFragmentStart !== undefined) candidate.historyFragmentStart = options.historyFragmentStart
          if (options.historyFragmentEnd !== undefined) candidate.historyFragmentEnd = options.historyFragmentEnd
          if (options.historyTotalChars !== undefined) candidate.historyTotalChars = options.historyTotalChars
          return
        }
      }
    }

    const last = entries.at(-1)
    if (!options.mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
      last.text += normalized
      return
    }

    nextId += 1
    const nextEntry: TranscriptEntry = { id: nextId, role, text: normalized }
    if (options.mergeKey) {
      nextEntry.mergeKey = options.mergeKey
    }
    if (options.sourceText !== undefined) nextEntry.sourceText = options.sourceText
    if (options.emphasis !== undefined) nextEntry.emphasis = options.emphasis
    if (options.historyEntryIndex !== undefined) nextEntry.historyEntryIndex = options.historyEntryIndex
    if (options.historyFragmentStart !== undefined) nextEntry.historyFragmentStart = options.historyFragmentStart
    if (options.historyFragmentEnd !== undefined) nextEntry.historyFragmentEnd = options.historyFragmentEnd
    if (options.historyTotalChars !== undefined) nextEntry.historyTotalChars = options.historyTotalChars
    if (options.turnId !== undefined) nextEntry.turnId = options.turnId
    entries.push(nextEntry)
  }

  for (const pageEntry of mergedHistoryEntries) {
    const options: {
      historyEntryIndex: number
      historyFragmentStart: number
      historyFragmentEnd: number
      historyTotalChars: number
      turnId?: number
    } = {
      historyEntryIndex: pageEntry.entry_index,
      historyFragmentStart: pageEntry.fragment_start,
      historyFragmentEnd: pageEntry.fragment_end,
      historyTotalChars: pageEntry.total_chars,
    }
    if (currentTurnId > 0) options.turnId = currentTurnId
    switch (pageEntry.entry.kind) {
      case "user_prompt":
        currentTurnId = Math.max(currentTurnId + 1, (pageEntry.entry_index ?? 0) + 1)
        appendTranscriptEntry("user", trimSingleTrailingNewline(pageEntry.entry.text), {
          ...options,
          turnId: currentTurnId,
        })
        break
      case "provider_reasoning":
        appendTranscriptEntry("reasoning", pageEntry.entry.text, {
          ...options,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
        })
        break
      case "provider_tool": {
        const parsed = parseToolTranscriptUpdate(pageEntry.entry.text)
        if (!parsed) {
          appendTranscriptEntry("tool", pageEntry.entry.text, {
            ...options,
            sourceText: pageEntry.entry.text,
          })
          break
        }
        const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
        tools.set(parsed.id, merged)
        appendTranscriptEntry("tool", formatToolTranscriptUpdate(merged), {
          ...options,
          mergeKey: parsed.id,
          sourceText: pageEntry.entry.text,
        })
        break
      }
      case "provider_error":
        appendTranscriptEntry("error", pageEntry.entry.text, {
          ...options,
          emphasis: "error",
        })
        break
      case "provider_status":
        if (shouldRenderProviderStatus(pageEntry.entry.text)) {
          appendTranscriptEntry("status", pageEntry.entry.text, {
            ...options,
            mergeKey: "__provider_status__",
          })
        }
        break
      case "notice":
        appendTranscriptEntry("notice", pageEntry.entry.text, options)
        break
      default:
        appendTranscriptEntry("assistant", pageEntry.entry.text, {
          ...options,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
        })
        break
    }
  }

  return markDeferredHistoryEntries(entries)
}

function appendPreviewLine(current: string, line: string) {
  const combined = current ? `${current}\n${line}` : line
  const lines = combined.split("\n")
  return lines.slice(-14).join("\n")
}

function formatHistoryPreview(historyEntries: SessionHistoryPageEntry[]) {
  const lines = mergeAdjacentHistoryPageEntries(historyEntries)
    .map((item) => previewLineForHistoryEntry(item.entry))
    .filter(Boolean) as string[]
  return lines.slice(-14).join("\n")
}

function formatTranscriptPreview(transcriptEntries: TranscriptEntry[]) {
  const lines = transcriptEntries
    .filter((entry) => entry && !entry.hidden)
    .map(previewLineForTranscriptEntry)
    .filter(Boolean) as string[]
  return lines.slice(-14).join("\n")
}

function previewLineForHistoryEntry(entry: SessionHistoryEntry) {
  const text = entry.text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  if (!text) {
    return null
  }
  const label = entry.kind === "user_prompt"
    ? "You"
    : entry.kind === "provider_reasoning"
      ? "Think"
      : entry.kind === "provider_tool"
        ? "Tool"
        : entry.kind === "provider_error"
          ? "Err"
          : entry.kind === "provider_status"
            ? "Stat"
            : entry.kind === "notice"
              ? "Note"
              : "Asst"
  return `${label}: ${text.split("\n")[0]}`
}

function previewLineForTranscriptEntry(entry: TranscriptEntry) {
  const text = entry.text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  if (!text || entry.role === "turn_toggle" || entry.role === "turn_summary") {
    return null
  }
  const label = entry.role === "user"
    ? "You"
    : entry.role === "reasoning"
      ? "Think"
      : entry.role === "tool"
        ? "Tool"
        : entry.role === "error"
          ? "Err"
          : entry.role === "status"
            ? "Stat"
            : entry.role === "notice"
              ? "Note"
              : "Asst"
  return `${label}: ${text.split("\n")[0]}`
}

function previewLineForTerminalRecord(kind: TerminalOutputRecord["kind"], text: string) {
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  if (!normalized) {
    return ""
  }
  const label = kind === "prompt_echo"
    ? "You"
    : kind === "provider_reasoning"
      ? "Think"
      : kind === "provider_tool"
        ? "Tool"
        : kind === "provider_error"
          ? "Err"
          : kind === "provider_status"
            ? "Stat"
            : "Asst"
  return `${label}: ${normalized.split("\n")[0]}`
}

function mergeAdjacentHistoryPageEntries(historyEntries: SessionHistoryPageEntry[]) {
  const merged: SessionHistoryPageEntry[] = []

  for (const entry of historyEntries) {
    const previous = merged.at(-1)
    if (
      previous
      && previous.entry_index === entry.entry_index
      && previous.entry.kind === entry.entry.kind
      && previous.fragment_end === entry.fragment_start
    ) {
      previous.fragment_end = entry.fragment_end
      previous.entry.text += entry.entry.text
      previous.total_chars = Math.max(previous.total_chars, entry.total_chars)
      continue
    }

    merged.push({
      entry_index: entry.entry_index,
      fragment_start: entry.fragment_start,
      fragment_end: entry.fragment_end,
      total_chars: entry.total_chars,
      entry: {
        kind: entry.entry.kind,
        text: entry.entry.text,
      },
    })
  }

  return merged
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
  prompt: string,
  attachments: PromptAttachmentPart[],
  options: CliOptions,
  logger?: ArrobaLogger | null,
): Promise<Record<string, unknown>> {
  try {
    return await client.send<Record<string, unknown>>(
      submitPromptRequest(sessionId, attachmentId, prompt, attachments),
    )
  } catch (error) {
    if (!isRecoverableProviderError(error)) {
      throw error
    }

    logger?.warn("prompt submission hit recoverable provider error", {
      error: formatError(error),
      session_id: sessionId,
    })
    await client.send<Record<string, unknown>>(
      launchProviderRunRequest(sessionId, options.accountProfile, options.model, options.effort),
    )
    await maybeResize(client, sessionId)
    logger?.info("relaunched provider after recoverable prompt failure", {
      session_id: sessionId,
    })
    return client.send<Record<string, unknown>>(
      submitPromptRequest(sessionId, attachmentId, prompt, attachments),
    )
  }
}

function isRecoverableProviderError(error: unknown): boolean {
  const message = formatError(error)
  return message.includes("has no active provider run") || message.includes("cannot perform `submit prompt` while ended")
}

function buildTranscriptEntryRenderable(
  renderer: ReturnType<typeof useRenderer>,
  entry: TranscriptEntry,
  transcriptSyntax: SyntaxStyle,
  onToggleTurn: (turnId: number | null | undefined, toggleEntryId?: number) => void,
  surfaceTone: TranscriptSurfaceTone = "default",
) {
  const patch = readTranscriptApplyPatch(entry)
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 1,
    flexDirection: "column",
  })
  const bodyColor = transcriptBodyColor(entry, surfaceTone)
  const body = new BoxRenderable(renderer, {
    paddingLeft: 1,
    paddingRight: 0,
    paddingTop: 1,
    paddingBottom: 1,
    ...(bodyColor ? { backgroundColor: bodyColor } : {}),
  })
  let update: (nextEntry: TranscriptEntry) => void

  if (patch) {
    buildApplyPatchTranscriptContent(renderer, body, patch, transcriptSyntax, surfaceTone)
    update = (nextEntry) => {
      for (const child of body.getChildren()) {
        body.remove(child.id)
        child.destroyRecursively()
      }
      const nextPatch = readTranscriptApplyPatch(nextEntry)
      if (nextPatch) {
        buildApplyPatchTranscriptContent(renderer, body, nextPatch, transcriptSyntax, surfaceTone)
        return
      }
      const markdown = new MarkdownRenderable(renderer, {
        content: normalizeMarkdownFenceInfoStrings(nextEntry.text),
        syntaxStyle: transcriptSyntax,
        conceal: true,
        concealCode: false,
        streaming: true,
      })
      body.add(markdown)
      markdown.requestRender()
    }
  } else if (shouldRenderTranscriptAsMarkdown(entry.role, entry.text)) {
    const markdown = new MarkdownRenderable(renderer, {
      content: normalizeMarkdownFenceInfoStrings(entry.text),
      syntaxStyle: transcriptSyntax,
      conceal: true,
      concealCode: false,
      streaming: true,
    })
    body.add(markdown)
    update = (nextEntry) => {
      markdown.content = normalizeMarkdownFenceInfoStrings(nextEntry.text)
      markdown.streaming = true
      markdown.requestRender()
    }
  } else {
    const text = new TextRenderable(renderer, {
      fg: transcriptTextColor(entry),
      wrapMode: "word",
    })
    if (entry.role === "turn_toggle") {
      text.onMouseUp = (event) => {
        if (event.button !== MouseButton.LEFT) {
          return
        }
        event.stopPropagation()
        startTimeout(() => {
          onToggleTurn(entry.turnId, entry.id)
        }, 0)
      }
    }
    applyTranscriptTextContent(text, entry)
    body.add(text)
    update = (nextEntry) => {
      applyTranscriptTextContent(text, nextEntry)
    }
  }

  if (transcriptUsesAccentBorder(entry)) {
    const border = new BoxRenderable(renderer, {
      border: ["left"],
      customBorderChars: SplitBorder.customBorderChars,
      borderColor: transcriptAccent(entry),
    })
    border.add(body)
    wrapper.add(border)
  } else {
    wrapper.add(body)
  }

  return { entry, wrapper, update }
}

function transcriptRenderMode(entry: TranscriptEntry) {
  if (readTranscriptApplyPatch(entry)) {
    return "patch"
  }
  if (shouldRenderTranscriptAsMarkdown(entry.role === "turn_summary" ? "assistant" : entry.role, entry.text)) {
    return "markdown"
  }
  return "text"
}

function readTranscriptApplyPatch(entry: TranscriptEntry) {
  const parsed = parseToolTranscriptUpdate(entry.sourceText ?? entry.text)
  if (!parsed) {
    return null
  }
  const files = readApplyPatchFiles(parsed)
  return files.length > 0 ? files : null
}

function buildApplyPatchTranscriptContent(
  renderer: ReturnType<typeof useRenderer>,
  body: BoxRenderable,
  files: ReturnType<typeof readApplyPatchFiles>,
  transcriptSyntax: SyntaxStyle,
  surfaceTone: TranscriptSurfaceTone,
) {
  const palette = transcriptSurfacePalette(surfaceTone)
  body.flexDirection = "column"
  body.gap = 1
  body.add(
    new TextRenderable(renderer, {
      content: `patch · ${files.length} ${files.length === 1 ? "file" : "files"}`,
      fg: theme.secondary,
      attributes: TextAttributes.BOLD,
      wrapMode: "word",
    }),
  )

  for (const file of files) {
    const block = new BoxRenderable(renderer, {
      flexDirection: "column",
      border: ["left"],
      customBorderChars: SplitBorder.customBorderChars,
      borderColor: theme.borderSubtle,
      paddingLeft: 1,
    })
    block.add(
      new TextRenderable(renderer, {
        content: file.title,
        fg: file.kind === "delete" ? theme.error : file.kind === "add" ? theme.success : theme.text,
        attributes: TextAttributes.BOLD,
        wrapMode: "word",
      }),
    )
    if (file.diff) {
      const diff = new DiffRenderable(renderer, {
        diff: file.diff,
        view: "split",
        filetype: guessPathFenceLanguage(file.filePath),
        syntaxStyle: transcriptSyntax,
        showLineNumbers: true,
        wrapMode: "none",
        fg: theme.text,
        addedBg: RGBA.fromHex("#102616"),
        removedBg: RGBA.fromHex("#2a1215"),
        contextBg: palette.element,
        addedSignColor: theme.success,
        removedSignColor: theme.error,
        lineNumberFg: theme.textMuted,
        lineNumberBg: palette.element,
        addedLineNumberBg: RGBA.fromHex("#16301d"),
        removedLineNumberBg: RGBA.fromHex("#34191d"),
      })
      block.add(
        diff,
      )
      startTimeout(() => {
        ;(diff as unknown as { requestRebuild?: () => void }).requestRebuild?.()
        diff.requestRender()
      }, 0)
    } else {
      block.add(
        new TextRenderable(renderer, {
          content: file.kind === "delete" ? "File deleted" : "No diff available",
          fg: theme.textMuted,
          wrapMode: "word",
        }),
      )
    }
    body.add(block)
  }
}

function appendAttachmentChip(text: TextRenderable, mime: string, filename: string) {
  const label = mime.startsWith("image/") ? "img" : mime === "application/pdf" ? "pdf" : "txt"
  const colors = mime.startsWith("image/")
    ? { accentBg: RGBA.fromHex("#f0d77d"), accentFg: RGBA.fromHex("#1f1400"), bodyBg: RGBA.fromHex("#2e2615") }
    : mime === "application/pdf"
      ? { accentBg: RGBA.fromHex("#8cc0ff"), accentFg: RGBA.fromHex("#09182b"), bodyBg: RGBA.fromHex("#172534") }
      : { accentBg: RGBA.fromHex("#8fd8a8"), accentFg: RGBA.fromHex("#0d1f13"), bodyBg: RGBA.fromHex("#173022") }
  text.add(TextNodeRenderable.fromString(` ${label} `, {
    fg: colors.accentFg,
    bg: colors.accentBg,
    attributes: TextAttributes.BOLD,
  }))
  text.add(TextNodeRenderable.fromString(` ${filename} `, {
    fg: theme.text,
    bg: colors.bodyBg,
    attributes: TextAttributes.BOLD,
  }))
}

function tokenMime(kind: string) {
  const value = kind.toLowerCase()
  if (value === "image") {
    return "image/png"
  }
  if (value === "pdf") {
    return "application/pdf"
  }
  return "text/plain"
}

function applyPromptTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  const lines = entry.text.split("\n")
  for (const [lineIndex, line] of lines.entries()) {
    appendPromptTranscriptLine(text, entry, line)
    if (lineIndex < lines.length - 1) {
      text.add("\n")
    }
  }
}

function appendPromptTranscriptLine(text: TextRenderable, entry: TranscriptEntry, line: string) {
  const matches = Array.from(line.matchAll(/\[(image|pdf|file)\s+(\d+)\]/gi))
  if (matches.length === 0) {
    appendTranscriptSpans(text, entry, line)
    return
  }
  let offset = 0
  for (const match of matches) {
    const index = match.index ?? 0
    if (index > offset) {
      appendTranscriptSpans(text, entry, line.slice(offset, index))
    }
    appendAttachmentChip(text, tokenMime(match[1] ?? "file"), `[${(match[1] ?? "file").toLowerCase()} ${match[2] ?? "1"}]`)
    offset = index + match[0].length
  }
  if (offset < line.length) {
    appendTranscriptSpans(text, entry, line.slice(offset))
  }
}

function appendTranscriptSpans(text: TextRenderable, entry: TranscriptEntry, value: string) {
  for (const span of splitInlineCodeSpans(value)) {
    text.add(
      TextNodeRenderable.fromString(
        span.text,
        span.code
          ? {
              fg: transcriptInlineCodeColor(entry),
              attributes: TextAttributes.BOLD,
            }
          : undefined,
      ),
    )
  }
}

function applyTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  text.clear()
  if (entry.role === "tool") {
    applyToolTranscriptTextContent(text, entry)
    return
  }
  if (entry.role === "user") {
    applyPromptTranscriptTextContent(text, entry)
    return
  }
  appendTranscriptSpans(text, entry, entry.text)
}

function applyToolTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  const newlineIndex = entry.text.indexOf("\n")
  const title = newlineIndex === -1 ? entry.text : entry.text.slice(0, newlineIndex)
  const rest = newlineIndex === -1 ? "" : entry.text.slice(newlineIndex)

  if (title) {
    text.add(TextNodeRenderable.fromString(title, { fg: theme.secondary }))
  }
  for (const span of splitInlineCodeSpans(rest)) {
    text.add(
      TextNodeRenderable.fromString(
        span.text,
        span.code
          ? {
              fg: transcriptInlineCodeColor(entry),
              attributes: TextAttributes.BOLD,
            }
          : {
              fg: theme.text,
            },
      ),
    )
  }
}

function buildEmptyTranscriptRenderable(renderer: ReturnType<typeof useRenderer>) {
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 0,
    flexGrow: 1,
    flexDirection: "column",
    justifyContent: "center",
    alignItems: "center",
    gap: 1,
  })
  wrapper.add(
    new TextRenderable(renderer, {
      content: arrobaArtFrame(12),
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "none",
    }),
  )
  wrapper.add(
    new TextRenderable(renderer, {
      content: "Type your first prompt below.",
      fg: theme.textMuted,
      wrapMode: "word",
    }),
  )
  return wrapper
}

function buildNoSessionRenderable(
  renderer: ReturnType<typeof useRenderer>,
  state: WaitingRoomState,
  sessions: RuntimeSession[],
  catalog: ProviderCatalog,
) {
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 0,
    flexGrow: 1,
    flexDirection: "column",
    justifyContent: "center",
    alignItems: "center",
    gap: 1,
  })
  const rows = waitingRoomRows(state, sessions, catalog)
  wrapper.add(
    new TextRenderable(renderer, {
      content: arrobaArtFrame(state.introStep),
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "none",
    }),
  )
  wrapper.add(
    new TextRenderable(renderer, {
      content: "No session attached. Dial in and choose your next run.",
      fg: theme.warning,
      wrapMode: "word",
    }),
  )
  const menu = new BoxRenderable(renderer, {
    flexDirection: "column",
    gap: 0,
    border: ["left"],
    borderColor: theme.secondary,
    customBorderChars: SplitBorder.customBorderChars,
    paddingLeft: 1,
  })
  for (const row of rows) {
    menu.add(
      new TextRenderable(renderer, {
        content: `${state.focus === row.id ? ">" : " "} ${row.title.padEnd(22, " ")} ${row.value}`,
        fg: state.focus === row.id ? theme.primary : theme.text,
        attributes: state.focus === row.id ? TextAttributes.BOLD : TextAttributes.NONE,
        wrapMode: "none",
      }),
    )
  }
  wrapper.add(menu)
  wrapper.add(
    new TextRenderable(renderer, {
      content: renderWaitingRoomKeys(state),
      fg: theme.textMuted,
      wrapMode: "none",
    }),
  )
  wrapper.add(
    new TextRenderable(renderer, {
      content: `${SESSION_NEW_HELP_TEXT}\nUse ↑ ↓ to move, ← → to cycle, Enter to confirm.`,
      fg: theme.textMuted,
      wrapMode: "word",
    }),
  )
  return wrapper
}

function buildDirectoryTreeRenderable(renderer: ReturnType<typeof useRenderer>, state: DirectoryTreeState) {
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 0,
    flexDirection: "column",
    gap: 0,
  })
  for (const row of buildDirectoryTreeRows(state)) {
    const isSelected = row.id === state.selectedPath
    const text = `${"  ".repeat(row.depth)}${row.kind === "root" || row.kind === "directory" ? (row.expanded ? "[-] " : "[+] ") : "    "}${row.label}`
    wrapper.add(
      new TextRenderable(renderer, {
        content: text,
        fg: row.kind === "root" ? theme.secondary : isSelected ? theme.primary : theme.text,
        ...(isSelected ? { bg: theme.backgroundElement } : {}),
        attributes: isSelected || row.kind === "root" ? TextAttributes.BOLD : TextAttributes.NONE,
        wrapMode: "none",
      }),
    )
  }
  return wrapper
}

function renderWaitingRoomKeys(state: WaitingRoomState) {
  const key = (label: string, pressed: boolean) => (pressed ? `[${label}]` : `<${label}>`)
  return [
    `        ${key("^", state.keyState.up)}`,
    `${key("<", state.keyState.left)} ${key("v", state.keyState.down)} ${key(">", state.keyState.right)}`,
  ].join("\n")
}

function computeCurrentTurnId(entries: TranscriptEntry[]) {
  return entries.reduce<number | null>((latest, entry) => {
    if (!entry || entry.role !== "user" || entry.turnId === undefined) {
      return latest
    }
    return entry.turnId
  }, null)
}

function computeNextTurnId(entries: TranscriptEntry[]) {
  return entries.reduce((max, entry) => Math.max(max, entry?.turnId ?? 0), 0) + 1
}

function transcriptAccent(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return theme.primary
  }
  if (entry.role === "reasoning") {
    return theme.accent
  }
  if (entry.role === "tool") {
    return theme.secondary
  }
  if (entry.role === "error") {
    return theme.error
  }
  if (entry.role === "status") {
    return theme.info
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? theme.error
      : entry.emphasis === "warning"
        ? theme.warning
        : theme.textMuted
  }
  if (entry.role === "turn_summary") {
    return theme.borderSubtle
  }
  if (entry.role === "turn_toggle") {
    return theme.info
  }
  return theme.borderSubtle
}

function isSessionUnavailableError(error: unknown): boolean {
  const message = formatError(error)
  return /session `[^`]+` was not found/i.test(message)
    || /attachment `[^`]+` was not found/i.test(message)
    || /does not belong to session/i.test(message)
    || /cannot perform `[^`]+` while ended/i.test(message)
}

function transcriptUsesAccentBorder(entry: TranscriptEntry) {
  return entry.role !== "status"
}

function resolveTranscriptSurfaceTone(splitActive: boolean, focused: boolean): TranscriptSurfaceTone {
  if (!splitActive) {
    return "default"
  }
  return focused ? "focused" : "faded"
}

function transcriptSurfacePalette(surfaceTone: TranscriptSurfaceTone) {
  if (surfaceTone === "focused") {
    return {
      panel: theme.backgroundPanel,
      element: theme.backgroundElement,
    }
  }
  if (surfaceTone === "faded") {
    return {
      panel: RGBA.fromHex("#171717"),
      element: RGBA.fromHex("#202020"),
    }
  }
  return {
    panel: theme.backgroundPanel,
    element: theme.backgroundElement,
  }
}

function transcriptBodyColor(entry: TranscriptEntry, surfaceTone: TranscriptSurfaceTone = "default") {
  const palette = transcriptSurfacePalette(surfaceTone)
  if (entry.role === "status") {
    return null
  }
  if (entry.role === "error") {
    return palette.panel
  }
  if (entry.role === "turn_summary") {
    return palette.panel
  }
  return entry.role === "assistant" || entry.role === "reasoning"
    ? palette.panel
    : palette.element
}

function transcriptTextColor(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return theme.text
  }
  if (entry.role === "reasoning") {
    return theme.textMuted
  }
  if (entry.role === "tool") {
    return theme.secondary
  }
  if (entry.role === "error") {
    return theme.error
  }
  if (entry.role === "status") {
    return theme.info
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? theme.error
      : entry.emphasis === "warning"
        ? theme.warning
        : theme.textMuted
  }
  if (entry.role === "turn_summary") {
    return theme.text
  }
  if (entry.role === "turn_toggle") {
    return theme.info
  }
  return theme.text
}

function transcriptInlineCodeColor(entry: TranscriptEntry) {
  if (entry.role === "tool" || entry.role === "status" || entry.role === "error" || entry.role === "turn_toggle") {
    return theme.primary
  }
  if (entry.role === "user") {
    return theme.text
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error" ? theme.warning : theme.info
  }
  return theme.info
}

function renderPromptTranscript(prompt: string) {
  const text = prompt.trimEnd()
  return text ? `${text}\n` : ""
}

function parseArgs(args: string[]): CliOptions {
  const options: CliOptions = {
    clientId: `arroba-cli-${process.pid}`,
    model: "default",
    accountProfile: "default",
    effort: "",
  }

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const next = () => {
      const value = args[index + 1]
      if (!value) {
        throw new Error(`missing value for ${arg}`)
      }
      index += 1
      return value
    }

    switch (arg) {
      case "--socket":
        options.socketPath = next()
        break
      case "--session":
        options.sessionId = next()
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
        throw new Error(`unknown argument ${arg}`)
    }
  }

  if (options.createSession && options.sessionId) {
    throw new Error("--create-session cannot be used together with --session")
  }
  if (options.createSession && options.deleteSessionRef) {
    throw new Error("--create-session cannot be used together with --delete-session")
  }
  if (options.alias && !options.createSession) {
    throw new Error("--alias requires --create-session")
  }

  return options
}

async function listSessions(client: LocalIpcClient): Promise<RuntimeSession[]> {
  const response = await client.send<Record<string, unknown>>(listSessionsRequest())
  const payload = expectVariant<{ sessions: RuntimeSession[] }>(response, "SessionsListed")
  return payload.sessions.sort((left, right) => right.created_at_ms - left.created_at_ms)
}

async function getProviderCatalog(client: LocalIpcClient, logger?: ArrobaLogger | null): Promise<ProviderCatalog> {
  try {
    const response = await client.send<Record<string, unknown>>(getProviderCatalogRequest())
    const payload = expectVariant<{ catalog: ProviderCatalog }>(response, "ProviderCatalog")
    logger?.info("Received provider catalog from daemon", {
      provider_count: payload.catalog.all.length,
      providers: payload.catalog.all.map((p) => ({ id: p.id, model_count: Object.keys(p.models).length })),
      connected: payload.catalog.connected,
    })
    return payload.catalog
  } catch (error) {
    logger?.warn("provider catalog lookup failed; using fallback catalog", {
      error: formatError(error),
    })
    return fallbackProviderCatalog()
  }
}

async function createSession(client: LocalIpcClient, workspace: string, worktree: string, alias?: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(createSessionRequest(workspace, worktree, alias))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionCreated")
  return payload.session
}

async function resolveSession(client: LocalIpcClient, sessionRef: string, workspace: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(resolveSessionRequest(sessionRef, workspace))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionResolved")
  return payload.session
}

async function deleteSessionByRef(client: LocalIpcClient, sessionRef: string, workspace: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(deleteSessionRequest(sessionRef, workspace))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionDeleted")
  return payload.session
}

async function attachToSession(
  client: LocalIpcClient,
  sessionId: string,
  clientId: string,
): Promise<RuntimeAttachment> {
  const response = await client.send<Record<string, unknown>>(attachToSessionRequest(sessionId, clientId))
  const payload = expectVariant<{ attachment: RuntimeAttachment }>(response, "SessionAttached")
  return payload.attachment
}

async function getProviderRun(client: LocalIpcClient, providerRunId: string): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId))
  const payload = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun")
  return payload.provider_run
}

async function tryGetProviderRun(
  client: LocalIpcClient,
  providerRunId: string,
  logger?: ArrobaLogger | null,
): Promise<RuntimeProviderRun | null> {
  try {
    return await getProviderRun(client, providerRunId)
  } catch (error) {
    const message = formatError(error)
    if (!/unknown variant `GetProviderRun`/i.test(message)) {
      throw error
    }
    logger?.warn("daemon does not support provider run lookup", {
      provider_run_id: providerRunId,
    })
    return null
  }
}

async function getSessionState(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
  return payload.session
}

async function updateSessionConfig(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  values: Record<string, string>,
  requiresIdle: boolean,
): Promise<{ session: RuntimeSession, config: SessionConfigState }> {
  const response = await client.send<Record<string, unknown>>(
    updateSessionConfigRequest(sessionId, attachmentId, values, requiresIdle),
  )
  return expectVariant<{ session: RuntimeSession, config: SessionConfigState }>(response, "SessionConfigUpdated")
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

async function catchUpAttachedSession(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  session: RuntimeSession,
  logger?: ArrobaLogger | null,
): Promise<void> {
  if (!session.active_provider_run_id) {
    return
  }

  try {
    await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
  } catch (error) {
    logger?.warn("attached session catch-up failed", {
      session_id: sessionId,
      attachment_id: attachmentId,
      error: formatError(error),
    })
  }
}

async function launchProviderRun(
  client: LocalIpcClient,
  sessionId: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId?: string | null,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(launchProviderRunRequest(sessionId, accountProfile, model, effort, agentId))
  const payload = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched")
  return payload.provider_run
}

async function maybeResize(client: LocalIpcClient, sessionId: string): Promise<void> {
  if (!process.stdout.isTTY || !process.stdout.columns || !process.stdout.rows) {
    return
  }
  await client.send<Record<string, unknown>>(resizeTerminalRequest(sessionId, process.stdout.columns, process.stdout.rows))
}

function sameProviderRun(left: RuntimeProviderRun, right: RuntimeProviderRun) {
  return left.id === right.id
    && left.session_id === right.session_id
    && left.agent_instance_id === right.agent_instance_id
    && left.adapter_key === right.adapter_key
    && left.provider === right.provider
    && left.account_profile === right.account_profile
    && left.model === right.model
    && left.variant === right.variant
    && left.usage_tokens_total === right.usage_tokens_total
    && left.state === right.state
}

function defaultSocketPath(): string {
  const daemonId = process.env.ARROBA_DAEMON_ID ?? "daemon-local"
  const runtimeDir = process.env.XDG_RUNTIME_DIR
    ? path.join(process.env.XDG_RUNTIME_DIR, "arroba")
    : homedir()
      ? path.join(homedir(), ".arroba", "run")
      : path.join(process.cwd(), ".arroba", "run")
  return process.env.ARROBA_DAEMON_SOCKET ?? path.join(runtimeDir, `${daemonId}.sock`)
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function firstVariantName(value: Record<string, unknown>): string {
  return Object.keys(value)[0] ?? "unknown"
}

function trimSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text
}

function formatError(error: unknown): string {
  return describeCliError(error)
}

function printUsage() {
  process.stdout.write(
    "usage: arroba-cli [--socket PATH] [--session REF] [--create-session] [--alias NAME] [--delete-session REF] [--client-id ID] [--model MODEL] [--account-profile PROFILE] [--effort LEVEL] [--workspace PATH] [--worktree PATH]\n       arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]\n\ncommands:\n  /stop                 request cancellation of the active provider turn\n  /exit                 exit the CLI\n  /waiting              go to the waiting room\n  /provider <name>      select the provider backend\n  /model <id>           select the active model\n  /variant <name>       select the model variant\n  /view <mode>          set multi-agent response layout to split|individual\n  /session new [a]      create and attach to a new session\n  /session create [a]   alias for /session new\n  /session attach <r>   attach to a session by id or alias\n  /session delete [r]   delete the current or referenced session\n  /agent spawn [a] [m]  spawn a new agent with optional alias and model\n  /agent delete [r]     delete the focused or referenced agent\n  /agent destroy [r]    alias for /agent delete\n  /agent focus <id>     focus a specific agent\n  /agent list           list all agents in the session\n  /agent cycle          cycle to the next agent (or use Ctrl+A)\n  Ctrl+A                keyboard shortcut to cycle to next agent\n",
  )
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
