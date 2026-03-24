import path from "node:path"
import process from "node:process"
import { homedir } from "node:os"
import { clearTimeout, setInterval as startInterval, setTimeout as startTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, DiffRenderable, MarkdownRenderable, MouseButton, RGBA, ScrollBoxRenderable, TextAttributes, TextNodeRenderable, TextRenderable, addDefaultParsers, parseKeypress, type KeyBinding, type SyntaxStyle, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { createSignal, onCleanup } from "solid-js"
import { createStore, produce, reconcile } from "solid-js/store"

import { copyTextToClipboard } from "./clipboard.js"
import { computePrependedHistoryScrollTop } from "./history-viewport.js"
import { LocalIpcClient } from "./ipc.js"
import { createProcessLogger, type ArrobaLogger } from "./logging.js"
import { runLogViewer } from "./logs.js"
import { loadPreferences, saveProviderPreferences } from "./preferences.js"
import { formatPromptMetaLine } from "./prompt-meta.js"
import {
  catalogModelOptions,
  fallbackProviderCatalog,
  selectConfiguredModel,
  selectConfiguredVariant,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  DEFAULT_CONNECTED_STATUS,
  describeCliError,
  getExitCleanupDecision,
  getPollRecoveryDecision,
  reconcileWorkingStateFromSession,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
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
import { createTranscriptSyntaxStyle, EmptyBorder, PromptBorderChars, SplitBorder, theme } from "./theme.js"
import {
  arrobaArtFrame,
  createWaitingRoomState,
  cycleWaitingRoomValue,
  moveWaitingRoomFocus,
  normalizeWaitingRoomState,
  waitingRoomChoice,
  waitingRoomRows,
  type WaitingRoomFocus,
  type WaitingRoomState,
} from "./waiting-room.js"
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
const STATUS_LABEL_WIDTH = "DISCONNECTED".length
const NO_SESSION_ID = "no-session"

type RuntimeSession = {
  id: string
  alias?: string | null
  workspace_id: string
  worktree_id: string
  created_at_ms: number
  status: string
  active_provider_run_id: string | null
  attachment_ids: string[]
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
}

type PromptQueueItem = {
  id: string
  source_attachment_id: string
  prompt: string
  status: string
}

type RuntimeAttachment = {
  id: string
  session_id: string
}

type RuntimeProviderRun = {
  id: string
  session_id: string
  adapter_key: string
  provider: string
  account_profile: string
  model: string
  variant: string | null
  state: string
}

type RuntimeNoticeRecord = {
  message: string
}

type TerminalOutputRecord = {
  kind: "provider_output" | "prompt_echo" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status"
  bytes: number[]
}

type PromptSubmittedPayload = {
  outcome: Record<string, unknown>
  session: RuntimeSession
}

type SessionHistoryPage = {
  entries: SessionHistoryPageEntry[]
  next_cursor: SessionHistoryCursor | null
}

type SessionHistoryCursor = {
  before_entry_index: number
  before_entry_char_offset: number | null
}

type SessionHistoryPageEntry = {
  entry_index: number
  fragment_start: number
  fragment_end: number
  total_chars: number
  entry: SessionHistoryEntry
}

type SessionHistoryEntry = {
  kind: "user_prompt" | "provider_output" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status" | "notice"
  text: string
}

type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "error" | "status" | "notice"
  text: string
  sourceText?: string
  mergeKey?: string
  emphasis?: "muted" | "warning" | "error"
  historyDeferred?: boolean
  historyEntryIndex?: number
  historyFragmentStart?: number
  historyFragmentEnd?: number
  historyTotalChars?: number
}

type TranscriptEntryRenderable = {
  entry: TranscriptEntry
  wrapper: BoxRenderable
  update: (entry: TranscriptEntry) => void
}

type CliOptions = {
  socketPath?: string
  sessionId?: string
  createSession?: boolean
  deleteSessionRef?: string
  alias?: string
  clientId: string
  model: string
  accountProfile: string
  effort: string
  workspace?: string
  worktree?: string
}

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

type BootstrapState = {
  client: LocalIpcClient
  binding: SessionBinding | null
  sessions: RuntimeSession[]
  providerCatalog: ProviderCatalog
  options: CliOptions
}

type SessionBinding = {
  session: RuntimeSession
  attachment: RuntimeAttachment
  providerRun: RuntimeProviderRun | null
  createdSession: boolean
  historyEntries: TranscriptEntry[]
  nextHistoryCursor: SessionHistoryCursor | null
}

type SessionStatusMode = "idle" | "working" | "disconnected"

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
  const bootstrap = await bootstrapSession(client, options, workspace, worktree)
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
  const [sessionState, setSessionState] = createSignal(initialSession)
  const [attachmentState, setAttachmentState] = createSignal<RuntimeAttachment | null>(initialBinding?.attachment ?? null)
  const [providerRunState, setProviderRunState] = createSignal<RuntimeProviderRun | null>(initialBinding?.providerRun ?? null)
  const [createdSessionState, setCreatedSessionState] = createSignal(initialBinding?.createdSession ?? false)
  const [availableSessions, setAvailableSessions] = createSignal<RuntimeSession[]>(initialSessions)
  const [providerCatalogState, setProviderCatalogState] = createSignal<ProviderCatalog>(initialProviderCatalog)
  const [waitingRoomState, setWaitingRoomState] = createSignal<WaitingRoomState>(
    createWaitingRoomState(initialSessions, initialProviderCatalog, options.model, options.effort),
  )
  const [entries, setEntries] = createStore<TranscriptEntry[]>(initialEntries)
  const [statusLine, setStatusLine] = createSignal(DEFAULT_CONNECTED_STATUS)
  const [fatalError, setFatalError] = createSignal<string | null>(null)
  const [submitting, setSubmitting] = createSignal(false)
  const [entryCounter, setEntryCounter] = createSignal(initialEntries.length)
  const [daemonDisconnected, setDaemonDisconnected] = createSignal(false)
  const [nextHistoryCursor, setNextHistoryCursor] = createSignal<SessionHistoryCursor | null>(initialBinding?.nextHistoryCursor ?? null)
  const [loadingHistory, setLoadingHistory] = createSignal(false)
  const [workingAnimationFrame, setWorkingAnimationFrame] = createSignal(0)
  const [working, setWorking] = createSignal(Boolean(initialSession.active_prompt) || initialSession.queued_prompts.length > 0)
  const [footerFlash, setFooterFlash] = createSignal<FooterFlash | null>(null)
  let stopRequestInFlight = false
  let promptInput: TextareaRenderable | undefined
  let transcriptScrollbox: ScrollBoxRenderable | undefined
  let promptStateBox: BoxRenderable | undefined
  let statusIndicatorBox: BoxRenderable | undefined
  let footerSummaryBox: BoxRenderable | undefined
  let historyLoadingBox: BoxRenderable | undefined
  let promptStateText: TextRenderable | undefined
  let promptMetaText: TextRenderable | undefined
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
  const transcriptRenderables = new Map<number, TranscriptEntryRenderable>()
  const transcriptSyntax = createTranscriptSyntaxStyle()
  let emptyTranscriptRenderable: BoxRenderable | undefined
  let footerFlashTimeout: ReturnType<typeof startTimeout> | undefined
  let lastTranscriptScrollTop = 0
  let historyLoadGeneration = 0
  let pendingHistoryScrollRestore = 0

  const isAttached = () => attachmentState() !== null
  const queueDepth = () => sessionState().queued_prompts.length
  const connectedClientCount = () => sessionState().attachment_ids.length
  const activePrompt = () => sessionState().active_prompt
  const logProviderRunDebug = (message: string, run: RuntimeProviderRun | null, fields: Record<string, unknown> = {}) => {
    appLogger?.debug(message, {
      provider_run_id: run?.id ?? null,
      provider: run?.provider ?? null,
      provider_model: run?.model ?? null,
      provider_variant: run?.variant ?? null,
      provider_state: run?.state ?? null,
      ...fields,
    })
  }
  const reconcileWaitingRoom = (next: WaitingRoomState) => {
    const previous = waitingRoomState()
    const normalized = normalizeWaitingRoomState(next, availableSessions(), providerCatalogState())
    setWaitingRoomState(normalized)
    options.model = normalized.modelId || options.model
    options.effort = normalized.effort
    if (options.model && (previous.modelId !== normalized.modelId || previous.effort !== normalized.effort)) {
      void saveProviderPreferences("opencode", {
        model: options.model,
        effort: options.effort,
      })
    }
    if (!isAttached()) {
      rebuildTranscript()
    }
    updateSessionChrome()
    return normalized
  }
  const activateWaitingRoom = async () => {
    const choice = waitingRoomChoice(waitingRoomState(), availableSessions(), providerCatalogState())
    const launch = {
      model: choice.model?.id ?? options.model,
      effort: choice.effort,
    }
    if (waitingRoomState().focus === "new") {
      const root = options.workspace ?? process.cwd()
      const session = await createSession(client, root, options.worktree ?? root)
      await attachBinding(session, true, launch)
      flashFooter(`created session ${session.alias ?? session.id}`, "info")
      return
    }
    if (waitingRoomState().focus === "join") {
      if (!choice.session) {
        flashFooter("no session available to join", "error")
        return
      }
      await attachBinding(choice.session as RuntimeSession, false, launch)
      flashFooter(`attached to session ${choice.session.alias ?? choice.session.id}`, "info")
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
  const promptMetaLine = () => {
    const run = providerRunState()
    const waiting = waitingRoomState()
    const provider = run?.provider ?? "opencode"
    const model = run?.model ?? waiting.modelId ?? options.model
    const effort = run?.variant ?? waiting.effort ?? options.effort
    return formatPromptMetaLine(provider, model, effort)
  }
  const hasPromptWork = (nextSession: RuntimeSession) => Boolean(nextSession.active_prompt) || nextSession.queued_prompts.length > 0
  const sessionStatusMode = (): SessionStatusMode => {
    if (daemonDisconnected()) {
      return "disconnected"
    }
    if (working() || activePrompt() || submitting() || queueDepth() > 0) {
      return "working"
    }
    return "idle"
  }
  const footerHint = () => {
    if (fatalError()) {
      return fatalError()!
    }
    if (activePrompt()) {
      return queueDepth() > 0
        ? `Processing ${activePrompt()!.id}; ${queueDepth()} queued.`
        : `Processing ${activePrompt()!.id}.`
    }
    return statusLine()
  }

  const appendEntry = (entry: Omit<TranscriptEntry, "id">) => {
    const nextId = entryCounter() + 1
    const nextEntry: TranscriptEntry = { id: nextId, ...entry }
    setEntryCounter(nextId)
    setEntries(entries.length, nextEntry)
    mountTranscriptEntry(nextEntry)
    enforceTranscriptRetention()
  }

  const scrollTranscriptToBottom = () => {
    if (!transcriptScrollbox) {
      return
    }
    transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: transcriptScrollbox.scrollHeight })
    transcriptScrollbox.requestRender()
  }

  const appendUserPrompt = (text: string) => {
    appendEntry({ role: "user", text: trimSingleTrailingNewline(text) })
    setSubmitting(true)
    setWorking(true)
    updateSessionChrome()
    scrollTranscriptToBottom()
  }

  const appendNotice = (text: string, emphasis: TranscriptEntry["emphasis"] = "muted") => {
    appendEntry({ role: "notice", text, emphasis })
    updateSessionChrome()
  }

  const appendProviderError = (text: string) => {
    const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
    if (!normalized) {
      return
    }
    setWorking(false)
    setSubmitting(false)
    appendEntry({ role: "error", text: normalized, emphasis: "error" })
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
    const nextHasPromptWork = hasPromptWork(nextSession)
    setSessionState(nextSession)
    setWorking(reconcileWorkingStateFromSession(working(), nextHasPromptWork))
    if (!nextSession.active_prompt) {
      setSubmitting(false)
      stopRequestInFlight = false
    }
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }

  const applyProviderActivity = (active: boolean) => {
    setWorking(active)
    if (!active) {
      setSubmitting(false)
      if (!activePrompt() && statusLine() === "Cancellation requested.") {
        setStatusLine(DEFAULT_CONNECTED_STATUS)
      }
    }
    updateSessionChrome()
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
    setWorking(true)
    setSubmitting(false)
    let mergedEntryId: number | undefined
    let mergedText: string | undefined
    let nextEntry: TranscriptEntry | undefined
    const nextId = entryCounter() + 1
    setEntries(
      produce((draft) => {
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
            existing.text = normalized
            if (normalizedSource !== undefined) existing.sourceText = normalizedSource
            mergedEntryId = existing.id
            mergedText = existing.text
            return
          }
        }
        const last = draft.at(-1)
        if (last?.role === role && (role === "assistant" || role === "reasoning")) {
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
    setWorking(true)
    updateSessionChrome()
    const parsed = parseToolTranscriptUpdate(normalized)
    if (parsed) {
      const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
      tools.set(parsed.id, merged)
      appendProviderChunk("tool", formatToolTranscriptUpdate(merged), parsed.id, JSON.stringify(merged))
      return
    }
    appendProviderChunk("tool", normalized, undefined, normalized)
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
      statusLabelTexts = Array.from({ length: STATUS_LABEL_WIDTH }, () => {
        const text = new TextRenderable(renderer, { wrapMode: "none" })
        statusIndicatorBox!.add(text)
        return text
      })
      statusIndicatorBox.add(statusCloseText)
    }
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

  const renderHistoryLoadingIndicator = () => {
    if (!historyLoadingBox) {
      return
    }
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
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }

  const setHistoryLoadingState = (next: boolean) => {
    setLoadingHistory(next)
    renderHistoryLoadingIndicator()
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
    const mode = sessionStatusMode()
    const label = mode === "working" ? "WORKING" : mode === "disconnected" ? "DISCONNECTED" : "IDLE"
    setTextRenderable(statusOpenText, "", theme.textMuted)
    for (let index = 0; index < STATUS_LABEL_WIDTH; index += 1) {
      const character = label[index] ?? " "
      let fg = theme.success
      if (mode === "disconnected") {
        fg = theme.error
      } else if (mode === "working") {
        const distance = reflectedDistance(index, label.length, workingAnimationFrame())
        fg = distance === 0 ? theme.primary : distance === 1 ? theme.warning : theme.secondary
      }
      setTextRenderable(
        statusLabelTexts[index],
        character,
        fg,
        mode === "working" && character.trim() ? TextAttributes.BOLD : TextAttributes.NONE,
      )
    }
    setTextRenderable(statusCloseText, "", theme.textMuted)
    statusIndicatorBox?.requestRender()
  }

  const updateSessionChrome = () => {
    ensureChromeRenderables()
    setTextRenderable(
      promptStateText,
      fatalError() ? "error" : submitting() ? "thinking" : footerHint(),
      fatalError() ? theme.error : submitting() ? theme.primary : theme.textMuted,
    )
    setTextRenderable(promptMetaText, isAttached() ? promptMetaLine() : " ", theme.textMuted)
    promptStateBox?.requestRender()
    setTextRenderable(
      footerSummaryText,
      isAttached()
        ? `Session ${sessionState().alias ?? sessionState().id} • ${connectedClientCount()} ${connectedClientCount() === 1 ? "CLI" : "CLIs"} connected • ${sessionState().active_provider_run_id ?? "starting provider"}${sessionStatusMode() === "working" ? " • Ctrl+C to stop agent" : ""}`
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
    ;(renderer as { requestRender?: () => void }).requestRender?.()
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

    const renderable = buildTranscriptEntryRenderable(renderer, entry, transcriptSyntax)
    transcriptRenderables.set(entry.id, renderable)
    transcriptScrollbox.add(renderable.wrapper)
    if (requestRender) {
      transcriptScrollbox.requestRender()
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
    transcriptScrollbox?.requestRender()
  }

  const rebuildTranscript = () => {
    if (!transcriptScrollbox) {
      return
    }

    for (const child of [...transcriptScrollbox.getChildren()]) {
      transcriptScrollbox.remove(child.id)
      child.destroyRecursively()
    }
    transcriptRenderables.clear()
    emptyTranscriptRenderable = undefined

    if (entries.length === 0) {
      emptyTranscriptRenderable = isAttached()
        ? buildEmptyTranscriptRenderable(renderer)
        : buildNoSessionRenderable(renderer, waitingRoomState(), availableSessions(), providerCatalogState())
      transcriptScrollbox.add(emptyTranscriptRenderable)
    } else {
      for (const entry of entries.filter((candidate) => candidate && !candidate.historyDeferred)) {
        mountTranscriptEntry(entry, false)
      }
    }

    transcriptScrollbox.requestRender()
  }

  const replaceTranscriptEntries = (nextEntries: TranscriptEntry[]) => {
    const sanitizedEntries = nextEntries.filter(Boolean)
    tools.clear()
    setEntries(reconcile(sanitizedEntries))
    setEntryCounter(sanitizedEntries.reduce((max, entry) => Math.max(max, entry.id), 0))
    rebuildTranscript()
    lastTranscriptScrollTop = transcriptScrollbox?.scrollTop ?? 0
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
    const nextCombinedEntries = stitchPrependedHistory(sanitizedEntries, currentEntries)
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

  const transitionToNoSession = (message = "No session attached.") => {
    setAttachmentState(null)
    setProviderRunState(null)
    setCreatedSessionState(false)
    setSessionState(buildDetachedSessionState(options))
    historyLoadGeneration += 1
    replaceTranscriptEntries([])
    setSubmitting(false)
    setWorking(false)
    stopRequestInFlight = false
    setFatalError(null)
    setDaemonDisconnected(false)
    setNextHistoryCursor(null)
    setHistoryLoadingState(false)
    setStatusLine(message)
    updateSessionChrome()
    promptInput?.clear()
    promptInput?.blur()
    reconcileWaitingRoom({
      ...waitingRoomState(),
      focus: "new",
      introStep: 0,
      keyState: { up: false, down: false, left: false, right: false },
    })
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
    session: RuntimeSession,
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
    historyLoadGeneration += 1
    const attachment = await attachToSession(client, session.id, options.clientId)
    const attachedSession = await getSessionState(client, session.id)
    if (!attachedSession.active_provider_run_id) {
      options.model = launch.model
      options.effort = launch.effort
      const run = await launchProviderRun(client, session.id, options.accountProfile, launch.model, launch.effort)
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
    await maybeResize(client, session.id)
    const historyPage = await getSessionHistory(client, session.id)
    const historyEntries = reindexTranscriptEntries(hydrateTranscriptEntries(historyPage.entries), 0)
    setAttachmentState(attachment)
    setCreatedSessionState(createdSession)
    setSessionState(await getSessionState(client, session.id))
    setNextHistoryCursor(historyPage.next_cursor)
    replaceTranscriptEntries(historyEntries)
    setFatalError(null)
    setDaemonDisconnected(false)
    setSubmitting(false)
    setWorking(hasPromptWork(sessionState()))
    setStatusLine(DEFAULT_CONNECTED_STATUS)
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
    try {
      const historyPage = await getSessionHistory(client, sessionId, cursor)
      if (generation !== historyLoadGeneration || !isAttached() || sessionState().id !== sessionId) {
        return
      }
      const hydratedEntries = reindexTranscriptEntries(hydrateTranscriptEntries(historyPage.entries), entryCounter())
      await prependTranscriptEntries(hydratedEntries)
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
    if (!trimmed) {
      promptInput.clear()
      return
    }
    if (trimmed === "/exit") {
      await requestExit()
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
      }
      return
    }
    if (trimmed === "/stop") {
      try {
        await requestPromptStop()
      } finally {
        promptInput.clear()
      }
      return
    }
    if (!isAttached()) {
      flashFooter(SESSION_NEW_ERROR_HINT, "error")
      promptInput.clear()
      return
    }

    const prompt = rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`
    try {
      appLogger?.info("submitting prompt", {
        chars: prompt.length,
      })
      const attachment = attachmentState()
      if (!attachment) {
        flashFooter("No session attached.", "error")
        promptInput.clear()
        return
      }
      const response = await submitPromptWithRecovery(client, sessionState().id, attachment.id, prompt, options, appLogger)
      const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
      applySessionState(payload.session)
      appendUserPrompt(prompt)
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
    if (event.ctrl && event.name === "c") {
      event.preventDefault()
      void (activePrompt() ? requestPromptStop() : requestExit())
    }
  })

  const handleSigint = () => {
    void (activePrompt() ? requestPromptStop() : requestExit())
  }
  const handleStdinData = (chunk: Buffer | string) => {
    const event = parseKeypress(chunk, { useKittyKeyboard: true })
    if (!event) {
      return
    }
    if (event?.ctrl && event.name === "c") {
      void (activePrompt() ? requestPromptStop() : requestExit())
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
      if (event.eventType === "press" && (event.name === "return" || event.name === "enter")) {
        void activateWaitingRoom()
      }
    }
  }
  process.on("SIGINT", handleSigint)
  process.stdin.on("data", handleStdinData)
  onCleanup(() => {
    process.off("SIGINT", handleSigint)
    process.stdin.off("data", handleStdinData)
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
      const response = await client.send<Record<string, unknown>>(
        pumpTerminalOutputRequest(sessionState().id, attachment.id),
      )
      const payload = expectVariant<{ records: TerminalOutputRecord[] }>(response, "TerminalOutput")
      for (const record of payload.records) {
        const text = Buffer.from(record.bytes).toString("utf8")
        switch (record.kind) {
          case "prompt_echo":
            appendEntry({ role: "user", text: trimSingleTrailingNewline(text) })
            break
          case "provider_reasoning":
            appendProviderChunk("reasoning", text)
            break
          case "provider_tool":
            appendToolUpdate(text)
            break
          case "provider_error":
            appendProviderError(text)
            break
          case "provider_status":
            applyProviderActivity(!/^OpenCode is idle\.?$/i.test(text.trim()))
            if (shouldRenderProviderStatus(text)) {
              appendProviderChunk("status", text, "__provider_status__")
            }
            break
          default:
            appendProviderChunk("assistant", text)
            break
        }
      }
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
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
      applySessionState(payload.session)
      if (payload.session.active_provider_run_id) {
        const activeRun = providerRunState()
        if (
          !activeRun
          || activeRun.id !== payload.session.active_provider_run_id
          || activeRun.model === "default"
          || activeRun.variant === null
        ) {
          const run = await tryGetProviderRun(client, payload.session.active_provider_run_id, appLogger)
          logProviderRunDebug("session poll refreshed provider run", run, {
            session_id: payload.session.id,
            previous_provider_run_id: activeRun?.id ?? null,
            previous_model: activeRun?.model ?? null,
            previous_variant: activeRun?.variant ?? null,
            refresh_reason: !activeRun
              ? "missing_run"
              : activeRun.id !== payload.session.active_provider_run_id
                ? "run_changed"
                : activeRun.model === "default"
                  ? "model_default"
                  : "variant_missing",
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
      }
    })
  }

  const ensureBackgroundPollersStarted = () => {
    if (pollersStarted) {
      return
    }
    if (!promptInput || !transcriptScrollbox) {
      return
    }
    pollersStarted = true
    rebuildTranscript()
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
  }

  onCleanup(() => {
    closing = true
    process.stdout.off("resize", onResize)
  })

  onCleanup(() => {
    if (footerFlashTimeout) {
      clearTimeout(footerFlashTimeout)
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
            historyLoadingBox = value
            renderHistoryLoadingIndicator()
          }}
          flexShrink={0}
          paddingLeft={2}
          paddingRight={1}
        />
        <scrollbox
          ref={(value) => {
            transcriptScrollbox = value
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
      </box>

      <box
        flexShrink={0}
        marginTop={1}
        border={["left"]}
        borderColor={fatalError() ? theme.error : theme.primary}
        customBorderChars={PromptBorderChars}
      >
        <box
          paddingLeft={2}
          paddingRight={2}
          paddingTop={1}
          paddingBottom={0}
          backgroundColor={theme.backgroundElement}
          flexDirection="column"
          gap={1}
        >
          <textarea
            ref={(value) => {
              promptInput = value
              ensureBackgroundPollersStarted()
            }}
            placeholder={isAttached() ? "Ask Arroba to do work in this session" : SESSION_NEW_PLACEHOLDER}
            textColor={theme.text}
            focusedTextColor={theme.text}
            minHeight={1}
            maxHeight={6}
            keyBindings={PROMPT_KEYBINDINGS}
            onSubmit={() => {
              void submitPrompt()
            }}
          />
          <text
            ref={(value) => {
              promptMetaText = value
              updateSessionChrome()
            }}
            fg={theme.textMuted}
          >
            {isAttached() ? promptMetaLine() : " "}
          </text>
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
    </box>
  )
}

function reflectedDistance(index: number, length: number, frame: number): number {
  if (length <= 1) {
    return 0
  }

  const span = length - 1
  const cycle = span * 2
  const position = frame % cycle
  const highlight = position <= span ? position : cycle - position
  return Math.abs(index - highlight)
}

function hydrateTranscriptEntries(historyEntries: SessionHistoryPageEntry[]): TranscriptEntry[] {
  const entries: TranscriptEntry[] = []
  const tools = new Map<string, ToolTranscriptUpdate>()
  let nextId = 0

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
          candidate.text = normalized
          if (options.sourceText !== undefined) candidate.sourceText = options.sourceText
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
    if (last?.role === role && (role === "assistant" || role === "reasoning")) {
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
    entries.push(nextEntry)
  }

  for (const pageEntry of historyEntries) {
    const options = {
      historyEntryIndex: pageEntry.entry_index,
      historyFragmentStart: pageEntry.fragment_start,
      historyFragmentEnd: pageEntry.fragment_end,
      historyTotalChars: pageEntry.total_chars,
    }
    switch (pageEntry.entry.kind) {
      case "user_prompt":
        appendTranscriptEntry("user", trimSingleTrailingNewline(pageEntry.entry.text), options)
        break
      case "provider_reasoning":
        appendTranscriptEntry("reasoning", pageEntry.entry.text, options)
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
        appendTranscriptEntry("assistant", pageEntry.entry.text, options)
        break
    }
  }

  return markDeferredHistoryEntries(entries)
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
  options: CliOptions,
  logger?: ArrobaLogger | null,
): Promise<Record<string, unknown>> {
  try {
    return await client.send<Record<string, unknown>>(
      submitPromptRequest(sessionId, attachmentId, prompt),
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
      submitPromptRequest(sessionId, attachmentId, prompt),
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
) {
  const patch = readTranscriptApplyPatch(entry)
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 1,
    flexDirection: "column",
  })
  const bodyColor = transcriptBodyColor(entry)
  const body = new BoxRenderable(renderer, {
    paddingLeft: 1,
    paddingRight: 0,
    paddingTop: 1,
    paddingBottom: 1,
    ...(bodyColor ? { backgroundColor: bodyColor } : {}),
  })
  let update: (nextEntry: TranscriptEntry) => void

  if (patch) {
    buildApplyPatchTranscriptContent(renderer, body, patch, transcriptSyntax)
    update = (nextEntry) => {
      for (const child of body.getChildren()) {
        body.remove(child.id)
        child.destroyRecursively()
      }
      const nextPatch = readTranscriptApplyPatch(nextEntry)
      if (nextPatch) {
        buildApplyPatchTranscriptContent(renderer, body, nextPatch, transcriptSyntax)
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
    }
  } else {
    const text = new TextRenderable(renderer, {
      fg: transcriptTextColor(entry),
      wrapMode: "word",
    })
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
  if (shouldRenderTranscriptAsMarkdown(entry.role, entry.text)) {
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
) {
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
        contextBg: theme.backgroundElement,
        addedSignColor: theme.success,
        removedSignColor: theme.error,
        lineNumberFg: theme.textMuted,
        lineNumberBg: theme.backgroundElement,
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

function applyTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  text.clear()
  if (entry.role === "tool") {
    applyToolTranscriptTextContent(text, entry)
    return
  }
  for (const span of splitInlineCodeSpans(entry.text)) {
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
  })
  wrapper.add(
    new TextRenderable(renderer, {
      content: "Type a prompt below. /stop cancels the active turn, /exit detaches from the session.",
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

function renderWaitingRoomKeys(state: WaitingRoomState) {
  const key = (label: string, pressed: boolean) => (pressed ? `[${label}]` : `<${label}>`)
  return [
    `        ${key("^", state.keyState.up)}`,
    `${key("<", state.keyState.left)} ${key("v", state.keyState.down)} ${key(">", state.keyState.right)}`,
  ].join("\n")
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
  return theme.borderSubtle
}

function buildDetachedSessionState(options: CliOptions): RuntimeSession {
  const workspace = options.workspace ?? process.cwd()
  const worktree = options.worktree ?? workspace
  return {
    id: NO_SESSION_ID,
    alias: null,
    workspace_id: workspace,
    worktree_id: worktree,
    created_at_ms: Date.now(),
    status: "Parked",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
  }
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

function transcriptBodyColor(entry: TranscriptEntry) {
  if (entry.role === "status") {
    return null
  }
  if (entry.role === "error") {
    return theme.backgroundPanel
  }
  return entry.role === "assistant" || entry.role === "reasoning"
    ? theme.backgroundPanel
    : theme.backgroundElement
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
  return theme.text
}

function transcriptInlineCodeColor(entry: TranscriptEntry) {
  if (entry.role === "tool" || entry.role === "status" || entry.role === "error") {
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

async function bootstrapSession(
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
): Promise<BootstrapState> {
  let createdSession = false
  let session: RuntimeSession | null = null

  const sessions = await listSessions(client)
  const providerCatalog = await getProviderCatalog(client, getLogger("cli.main"))
  const decision = decideBootstrapAction(options, sessions, workspace, worktree)
  switch (decision.action) {
    case "create":
      session = await createSession(client, workspace, worktree, options.alias)
      createdSession = true
      break
    case "resolve":
      session = await resolveSession(client, decision.sessionRef, workspace)
      break
    case "attach_existing": {
      const existing = selectAttachableSession(sessions, workspace, worktree)
      if (!existing) {
        session = await createSession(client, workspace, worktree, options.alias)
        createdSession = true
        break
      }
      session = existing as RuntimeSession
      break
    }
    case "none":
      return {
        client,
        binding: null,
        sessions,
        providerCatalog,
        options,
      }
  }

  if (!session) {
    return {
      client,
      binding: null,
      sessions,
      providerCatalog,
      options,
    }
  }

  const attachment = await attachToSession(client, session.id, options.clientId)
  const attachedSession = await getSessionState(client, session.id)
  let providerRun: RuntimeProviderRun | null = null
  if (!attachedSession.active_provider_run_id) {
    providerRun = await launchProviderRun(client, session.id, options.accountProfile, options.model, options.effort)
  } else {
    providerRun = await tryGetProviderRun(client, attachedSession.active_provider_run_id)
  }
  const historyPage = await getSessionHistory(client, session.id)
  const historyEntries = reindexTranscriptEntries(hydrateTranscriptEntries(historyPage.entries), 0)

  return {
    client,
    binding: {
      session: await getSessionState(client, session.id),
      attachment,
      providerRun,
      createdSession,
      historyEntries,
      nextHistoryCursor: historyPage.next_cursor,
    },
    sessions,
    providerCatalog,
    options,
  }
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

async function getSessionHistory(client: LocalIpcClient, sessionId: string, cursor?: SessionHistoryCursor | null): Promise<SessionHistoryPage> {
  const response = await client.send<Record<string, unknown>>(getSessionHistoryRequest(sessionId, cursor))
  return expectVariant<SessionHistoryPage>(response, "SessionHistory")
}

async function launchProviderRun(
  client: LocalIpcClient,
  sessionId: string,
  accountProfile: string,
  model: string,
  effort: string,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(launchProviderRunRequest(sessionId, accountProfile, model, effort))
  const payload = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched")
  return payload.provider_run
}

async function maybeResize(client: LocalIpcClient, sessionId: string): Promise<void> {
  if (!process.stdout.isTTY || !process.stdout.columns || !process.stdout.rows) {
    return
  }
  await client.send<Record<string, unknown>>(resizeTerminalRequest(sessionId, process.stdout.columns, process.stdout.rows))
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

function createSessionRequest(workspaceId: string, worktreeId: string, alias?: string) {
  return {
    CreateSession: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      alias: alias ?? null,
    },
  }
}

function listSessionsRequest() {
  return { ListSessions: null }
}

function resolveSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    ResolveSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

function attachToSessionRequest(sessionId: string, clientId: string) {
  return {
    AttachToSession: {
      session_id: sessionId,
      client_id: clientId,
      capability_level: "FullTerminal",
    },
  }
}

function detachFromSessionRequest(attachmentId: string) {
  return {
    DetachFromSession: {
      attachment_id: attachmentId,
    },
  }
}

function endSessionRequest(sessionId: string) {
  return {
    EndSession: {
      session_id: sessionId,
    },
  }
}

function deleteSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    DeleteSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

function getSessionStateRequest(sessionId: string) {
  return {
    GetSessionState: {
      session_id: sessionId,
    },
  }
}

function getProviderRunRequest(providerRunId: string) {
  return {
    GetProviderRun: {
      provider_run_id: providerRunId,
    },
  }
}

function getProviderCatalogRequest() {
  return {
    GetProviderCatalog: {},
  }
}

function getSessionHistoryRequest(sessionId: string, cursor?: SessionHistoryCursor | null) {
  return {
    GetSessionHistory: {
      session_id: sessionId,
      round_count: HISTORY_PAGE_ROUND_COUNT,
      max_chars: BOOTSTRAP_HISTORY_MAX_CHARS,
      before_entry_index: cursor?.before_entry_index ?? null,
      before_entry_char_offset: cursor?.before_entry_char_offset ?? null,
    },
  }
}

function launchProviderRunRequest(sessionId: string, accountProfile: string, model: string, effort: string) {
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: accountProfile,
      model,
      variant: effort.trim() || null,
    },
  }
}

function resizeTerminalRequest(sessionId: string, cols: number, rows: number) {
  return {
    ResizeTerminal: {
      session_id: sessionId,
      cols,
      rows,
    },
  }
}

function pumpTerminalOutputRequest(sessionId: string, attachmentId: string) {
  return {
    PumpTerminalOutput: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

function submitPromptRequest(sessionId: string, attachmentId: string, prompt: string) {
  return {
    SubmitPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      prompt,
    },
  }
}

function cancelActivePromptRequest(sessionId: string, attachmentId: string) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

function pollRuntimeNoticesRequest(sessionId: string, attachmentId: string) {
  return {
    PollRuntimeNotices: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
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
    "usage: arroba-cli [--socket PATH] [--session REF] [--create-session] [--alias NAME] [--delete-session REF] [--client-id ID] [--model MODEL] [--account-profile PROFILE] [--effort LEVEL] [--workspace PATH] [--worktree PATH]\n       arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]\n\ncommands:\n  /stop                 request cancellation of the active provider turn\n  /exit                 exit the CLI\n  /session new [a]      create and attach to a new session\n  /session create [a]   alias for /session new\n  /session attach <r>   attach to a session by id or alias\n  /session delete [r]   delete the current or referenced session\n",
  )
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
