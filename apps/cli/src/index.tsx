import path from "node:path"
import process from "node:process"
import { homedir } from "node:os"
import { setInterval as startInterval } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, ScrollBoxRenderable, TextAttributes, TextNodeRenderable, TextRenderable, parseKeypress, type KeyBinding, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { createSignal, onCleanup } from "solid-js"
import { createStore, produce } from "solid-js/store"

import { LocalIpcClient } from "./ipc.js"
import { createProcessLogger, type ArrobaLogger } from "./logging.js"
import { runLogViewer } from "./logs.js"
import {
  DEFAULT_CONNECTED_STATUS,
  describeCliError,
  getExitCleanupDecision,
  getPollRecoveryDecision,
  shouldEndSessionOnCliExit,
} from "./runtime.js"
import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  splitInlineCodeSpans,
  shouldRenderProviderStatus,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import { EmptyBorder, PromptBorderChars, SplitBorder, theme } from "./theme.js"

const PROMPT_KEYBINDINGS = [
  { name: "return", action: "submit" },
  { name: "return", meta: true, action: "newline" },
] satisfies KeyBinding[]

const BOOTSTRAP_HISTORY_LIMIT = 200
const BOOTSTRAP_HISTORY_MAX_CHARS = 250_000

type RuntimeSession = {
  id: string
  workspace_id: string
  worktree_id: string
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

type RuntimeNoticeRecord = {
  message: string
}

type TerminalOutputRecord = {
  kind: "provider_output" | "prompt_echo" | "provider_reasoning" | "provider_tool" | "provider_status"
  bytes: number[]
}

type PromptSubmittedPayload = {
  outcome: Record<string, unknown>
  session: RuntimeSession
}

type SessionHistoryEntry = {
  kind: "user_prompt" | "provider_output" | "provider_reasoning" | "provider_tool" | "provider_status" | "notice"
  text: string
}

type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "status" | "notice"
  text: string
  mergeKey?: string
  emphasis?: "muted" | "warning" | "error"
}

type TranscriptEntryRenderable = {
  entry: TranscriptEntry
  wrapper: BoxRenderable
  text: TextRenderable
}

type CliOptions = {
  socketPath?: string
  sessionId?: string
  clientId: string
  model: string
  accountProfile: string
  workspace?: string
  worktree?: string
}

type BootstrapState = {
  client: LocalIpcClient
  session: RuntimeSession
  attachment: RuntimeAttachment
  createdSession: boolean
  historyEntries: TranscriptEntry[]
  options: CliOptions
}

type SessionStatusMode = "idle" | "working" | "disconnected"

const OPEN_CONSOLE_ON_ERROR = (process.env.ARROBA_LOG_LEVEL ?? "").toLowerCase() === "debug"
let processLogger: ArrobaLogger | null = null

function getLogger(component: string, fields: Record<string, unknown> = {}) {
  return processLogger?.child(component, fields) ?? null
}

async function main() {
  const argv = process.argv.slice(2)
  if (argv[0] === "logs") {
    await runLogViewer(argv.slice(1))
    return
  }

  processLogger = createProcessLogger("cli")
  getLogger("cli.main")?.info("starting cli process", { argv })
  const options = parseArgs(argv)
  const socketPath = options.socketPath ?? defaultSocketPath()
  const client = new LocalIpcClient(socketPath)
  const workspace = options.workspace ?? process.cwd()
  const worktree = options.worktree ?? workspace
  getLogger("cli.main")?.info("bootstrapping cli session", {
    socket_path: socketPath,
    workspace_id: workspace,
    worktree_id: worktree,
    client_id: options.clientId,
  })
  const bootstrap = await bootstrapSession(client, options, workspace, worktree)
  getLogger("cli.main")?.info("bootstrapped cli session", {
    session_id: bootstrap.session.id,
    attachment_id: bootstrap.attachment.id,
    created_session: bootstrap.createdSession,
  })
  await maybeResize(client, bootstrap.session.id)
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

function ArrobaCliApp(props: { bootstrap: BootstrapState }) {
  const { client, session, attachment, createdSession, options } = props.bootstrap
  const appLogger = getLogger("cli.app", {
    session_id: session.id,
    attachment_id: attachment.id,
    client_id: options.clientId,
  })
  const renderer = useRenderer()
  const dimensions = useTerminalDimensions()
  const initialEntries = props.bootstrap.historyEntries
  const [sessionState, setSessionState] = createSignal(session)
  const [entries, setEntries] = createStore<TranscriptEntry[]>(initialEntries)
  const [statusLine, setStatusLine] = createSignal(DEFAULT_CONNECTED_STATUS)
  const [fatalError, setFatalError] = createSignal<string | null>(null)
  const [submitting, setSubmitting] = createSignal(false)
  const [entryCounter, setEntryCounter] = createSignal(initialEntries.length)
  const [daemonDisconnected, setDaemonDisconnected] = createSignal(false)
  const [workingAnimationFrame, setWorkingAnimationFrame] = createSignal(0)
  const [working, setWorking] = createSignal(Boolean(session.active_prompt) || session.queued_prompts.length > 0)
  let promptInput: TextareaRenderable | undefined
  let transcriptScrollbox: ScrollBoxRenderable | undefined
  let headerMetaBox: BoxRenderable | undefined
  let promptStateBox: BoxRenderable | undefined
  let statusIndicatorBox: BoxRenderable | undefined
  let footerSummaryBox: BoxRenderable | undefined
  let closing = false
  let exitCleanupFailed = false
  const degradedPollers = new Set<string>()
  const tools = new Map<string, ToolTranscriptUpdate>()
  const transcriptRenderables = new Map<number, TranscriptEntryRenderable>()
  let emptyTranscriptRenderable: BoxRenderable | undefined

  const queueDepth = () => sessionState().queued_prompts.length
  const connectedClientCount = () => sessionState().attachment_ids.length
  const activePrompt = () => sessionState().active_prompt
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

  const applySessionState = (nextSession: RuntimeSession) => {
    setSessionState(nextSession)
    setWorking(Boolean(nextSession.active_prompt) || nextSession.queued_prompts.length > 0)
    if (!nextSession.active_prompt) {
      setSubmitting(false)
    }
    updateSessionChrome()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }

  const applyProviderActivity = (active: boolean) => {
    setWorking(active)
    if (!active) {
      setSubmitting(false)
    }
    updateSessionChrome()
  }

  const appendProviderChunk = (role: TranscriptEntry["role"], chunk: string, mergeKey?: string) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
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
        draft.push(nextEntry)
      }),
    )
    if (mergedEntryId !== undefined && mergedText !== undefined) {
      updateTranscriptEntry(mergedEntryId, mergedText)
      return
    }
    if (!nextEntry) {
      return
    }
    setEntryCounter(nextId)
    mountTranscriptEntry(nextEntry)
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
      appendProviderChunk("tool", formatToolTranscriptUpdate(merged), parsed.id)
      return
    }
    appendProviderChunk("tool", normalized)
  }

  const renderStatusIndicator = () => {
    if (!statusIndicatorBox) {
      return
    }

    for (const child of [...statusIndicatorBox.getChildren()]) {
      statusIndicatorBox.remove(child.id)
    }

    statusIndicatorBox.add(new TextRenderable(renderer, { content: "[", fg: theme.textMuted }))

    const mode = sessionStatusMode()
    const label = mode === "working" ? "WORKING" : mode === "disconnected" ? "DISCONNECTED" : "IDLE"
    for (const [index, character] of [...label].entries()) {
      let fg = theme.success
      if (mode === "disconnected") {
        fg = theme.error
      } else if (mode === "working") {
        const distance = reflectedDistance(index, label.length, workingAnimationFrame())
        fg = distance === 0 ? theme.primary : distance === 1 ? theme.warning : theme.secondary
      }
      statusIndicatorBox.add(
        new TextRenderable(renderer, {
          content: character,
          fg,
          attributes: mode === "working" ? TextAttributes.BOLD : TextAttributes.NONE,
        }),
      )
    }

    statusIndicatorBox.add(new TextRenderable(renderer, { content: "]", fg: theme.textMuted }))
    statusIndicatorBox.requestRender()
  }

  const replaceBoxText = (
    box: BoxRenderable | undefined,
    parts: Array<{ content: string; fg: (typeof theme)[keyof typeof theme]; attributes?: number }>,
  ) => {
    if (!box) {
      return
    }
    for (const child of [...box.getChildren()]) {
      box.remove(child.id)
    }
    for (const part of parts) {
      box.add(
        new TextRenderable(renderer, {
          content: part.content,
          fg: part.fg,
          attributes: part.attributes ?? TextAttributes.NONE,
        }),
      )
    }
    box.requestRender()
  }

  const updateSessionChrome = () => {
    replaceBoxText(headerMetaBox, [
      {
        content: `${connectedClientCount()} ${connectedClientCount() === 1 ? "CLI" : "CLIs"} connected`,
        fg: connectedClientCount() > 1 ? theme.info : theme.textMuted,
      },
      {
        content: sessionState().active_provider_run_id ?? "starting provider",
        fg: theme.textMuted,
      },
    ])
    replaceBoxText(promptStateBox, [
      {
        content: fatalError() ? "error" : submitting() ? "thinking" : footerHint(),
        fg: fatalError() ? theme.error : submitting() ? theme.primary : theme.textMuted,
      },
    ])
    replaceBoxText(footerSummaryBox, [
      {
        content: `Session ${sessionState().id} • ${queueDepth()} queued • Enter sends • Meta+Enter adds newline • Ctrl+C or /exit to leave`,
        fg: theme.textMuted,
      },
    ])
    renderStatusIndicator()
    ;(renderer as { requestRender?: () => void }).requestRender?.()
  }

  const mountTranscriptEntry = (entry: TranscriptEntry, requestRender = true) => {
    if (!transcriptScrollbox) {
      return
    }

    if (emptyTranscriptRenderable) {
      transcriptScrollbox.remove(emptyTranscriptRenderable.id)
      emptyTranscriptRenderable = undefined
    }

    const renderable = buildTranscriptEntryRenderable(renderer, entry)
    transcriptRenderables.set(entry.id, renderable)
    transcriptScrollbox.add(renderable.wrapper)
    if (requestRender) {
      transcriptScrollbox.requestRender()
    }
  }

  const updateTranscriptEntry = (entryId: number, text: string) => {
    const renderable = transcriptRenderables.get(entryId)
    if (!renderable) {
      rebuildTranscript()
      return
    }
    renderable.entry.text = text
    applyTranscriptTextContent(renderable.text, renderable.entry)
    transcriptScrollbox?.requestRender()
  }

  const rebuildTranscript = () => {
    if (!transcriptScrollbox) {
      return
    }

    for (const child of [...transcriptScrollbox.getChildren()]) {
      transcriptScrollbox.remove(child.id)
    }
    transcriptRenderables.clear()
    emptyTranscriptRenderable = undefined

    if (entries.length === 0) {
      emptyTranscriptRenderable = buildEmptyTranscriptRenderable(renderer)
      transcriptScrollbox.add(emptyTranscriptRenderable)
    } else {
      for (const entry of entries) {
        mountTranscriptEntry(entry, false)
      }
    }

    transcriptScrollbox.requestRender()
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
      created_session: createdSession,
    })
    try {
      if (shouldEndSessionOnCliExit(createdSession, connectedClientCount())) {
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
    if (trimmed === "/stop") {
      try {
        await client.send(cancelActivePromptRequest(sessionState().id, attachment.id))
        appLogger?.info("requested active prompt cancellation")
        setStatusLine("Cancellation requested.")
        setWorking(true)
        updateSessionChrome()
      } catch (error) {
        appLogger?.error("active prompt cancellation failed", {
          error: formatError(error),
        })
        setFatalError(formatError(error))
        updateSessionChrome()
      } finally {
        promptInput.clear()
      }
      return
    }

    const prompt = rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`
    try {
      appLogger?.info("submitting prompt", {
        chars: prompt.length,
      })
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

  useKeyboard((event) => {
    if (event.ctrl && event.name === "c") {
      event.preventDefault()
      void requestExit()
    }
  })

  const handleSigint = () => {
    void requestExit()
  }
  const handleStdinData = (chunk: Buffer | string) => {
    const event = parseKeypress(chunk, { useKittyKeyboard: true })
    if (event?.ctrl && event.name === "c") {
      void requestExit()
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
    void maybeResize(client, sessionState().id)
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
          case "provider_status":
            applyProviderActivity(!/^OpenCode is idle\.?$/i.test(text.trim()))
            if (shouldRenderProviderStatus(text)) {
              appendProviderChunk("status", text)
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
      const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionState().id))
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
      applySessionState(payload.session)
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
    promptInput?.focus()
    rebuildTranscript()
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

  const workingAnimation = startInterval(() => {
    setWorkingAnimationFrame((value) => value + 1)
    if (sessionStatusMode() === "working") {
      updateSessionChrome()
    }
  }, 120)

  onCleanup(() => {
    clearInterval(workingAnimation)
  })

  return (
    <box
      width={dimensions().width}
      height={dimensions().height}
      flexDirection="column"
      paddingTop={1}
      paddingBottom={1}
      paddingLeft={2}
      paddingRight={2}
      backgroundColor={theme.background}
    >
      <box
        flexShrink={0}
        paddingLeft={2}
        paddingRight={2}
        paddingTop={1}
        paddingBottom={1}
        backgroundColor={theme.backgroundPanel}
        border={["left", "right"]}
        customBorderChars={SplitBorder.customBorderChars}
        borderColor={theme.borderSubtle}
      >
        <box flexDirection="row" justifyContent="space-between">
          <box flexDirection="column" gap={0}>
            <text attributes={TextAttributes.BOLD} fg={theme.text}>
              Arroba CLI
            </text>
            <text fg={theme.textMuted}>
              Session {sessionState().id} on {path.basename(sessionState().worktree_id) || sessionState().worktree_id}
            </text>
          </box>
          <box
            ref={(value) => {
              headerMetaBox = value
              updateSessionChrome()
            }}
            flexDirection="column"
            alignItems="flex-end"
            justifyContent="center"
          />
        </box>
      </box>

      <box
        flexGrow={1}
        marginTop={1}
        backgroundColor={theme.backgroundPanel}
        border={["left", "right"]}
        customBorderChars={SplitBorder.customBorderChars}
        borderColor={theme.borderSubtle}
      >
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
          paddingBottom={1}
          backgroundColor={theme.backgroundElement}
          flexDirection="column"
          gap={1}
        >
          <box flexDirection="row" justifyContent="space-between">
            <text fg={theme.textMuted}>Prompt</text>
            <box
              ref={(value) => {
                promptStateBox = value
                updateSessionChrome()
              }}
              flexDirection="row"
            />
          </box>
          <textarea
            ref={(value) => {
              promptInput = value
              ensureBackgroundPollersStarted()
            }}
            placeholder="Ask Arroba to do work in this session"
            textColor={theme.text}
            focusedTextColor={theme.text}
            minHeight={1}
            maxHeight={6}
            keyBindings={PROMPT_KEYBINDINGS}
            onSubmit={() => {
              void submitPrompt()
            }}
          />
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

function hydrateTranscriptEntries(historyEntries: SessionHistoryEntry[]): TranscriptEntry[] {
  const entries: TranscriptEntry[] = []
  const tools = new Map<string, ToolTranscriptUpdate>()
  let nextId = 0

  const appendTranscriptEntry = (role: TranscriptEntry["role"], chunk: string, mergeKey?: string) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }

    if (mergeKey) {
      for (let index = entries.length - 1; index >= 0; index -= 1) {
        const candidate = entries[index]
        if (candidate?.role === role && candidate.mergeKey === mergeKey) {
          candidate.text = normalized
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
    if (mergeKey) {
      nextEntry.mergeKey = mergeKey
    }
    entries.push(nextEntry)
  }

  for (const entry of historyEntries) {
    switch (entry.kind) {
      case "user_prompt":
        appendTranscriptEntry("user", trimSingleTrailingNewline(entry.text))
        break
      case "provider_reasoning":
        appendTranscriptEntry("reasoning", entry.text)
        break
      case "provider_tool": {
        const parsed = parseToolTranscriptUpdate(entry.text)
        if (!parsed) {
          appendTranscriptEntry("tool", entry.text)
          break
        }
        const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
        tools.set(parsed.id, merged)
        appendTranscriptEntry("tool", formatToolTranscriptUpdate(merged), parsed.id)
        break
      }
      case "provider_status":
        if (shouldRenderProviderStatus(entry.text)) {
          appendTranscriptEntry("status", entry.text)
        }
        break
      case "notice":
        appendTranscriptEntry("notice", entry.text)
        break
      default:
        appendTranscriptEntry("assistant", entry.text)
        break
    }
  }

  return entries
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
      launchProviderRunRequest(sessionId, options.accountProfile, options.model),
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

function buildTranscriptEntryRenderable(renderer: ReturnType<typeof useRenderer>, entry: TranscriptEntry) {
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
  const text = new TextRenderable(renderer, {
    fg: transcriptTextColor(entry),
    wrapMode: "word",
  })
  applyTranscriptTextContent(text, entry)
  body.add(text)

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

  return { entry, wrapper, text }
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

function transcriptAccent(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return theme.primary
  }
  if (entry.role === "reasoning") {
    return theme.accent
  }
  if (entry.role === "tool") {
    return theme.text
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

function transcriptUsesAccentBorder(entry: TranscriptEntry) {
  return entry.role !== "status"
}

function transcriptBodyColor(entry: TranscriptEntry) {
  if (entry.role === "status") {
    return null
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
  if (entry.role === "tool" || entry.role === "status") {
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
      case "--client-id":
        options.clientId = next()
        break
      case "--model":
        options.model = next()
        break
      case "--account-profile":
        options.accountProfile = next()
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

  return options
}

async function bootstrapSession(
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
): Promise<BootstrapState> {
  let createdSession = false
  let session: RuntimeSession

  if (options.sessionId) {
    session = await getSessionState(client, options.sessionId)
  } else {
    const existing = await findAttachableSession(client, workspace, worktree)
    if (existing) {
      session = existing
    } else {
      session = await createSession(client, workspace, worktree)
      createdSession = true
    }
  }

  const attachment = await attachToSession(client, session.id, options.clientId)
  const attachedSession = await getSessionState(client, session.id)
  if (!attachedSession.active_provider_run_id) {
    await launchProviderRun(client, session.id, options.accountProfile, options.model)
  }
  const historyEntries = hydrateTranscriptEntries(await getSessionHistory(client, session.id))

  return {
    client,
    session: await getSessionState(client, session.id),
    attachment,
    createdSession,
    historyEntries,
    options,
  }
}

async function findAttachableSession(
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
): Promise<RuntimeSession | null> {
  const response = await client.send<Record<string, unknown>>(listSessionsRequest())
  const payload = expectVariant<{ sessions: RuntimeSession[] }>(response, "SessionsListed")
  return payload.sessions
    .filter((session) => session.workspace_id === workspace && session.worktree_id === worktree && session.status !== "Ended")
    .sort((left, right) => sessionNumber(right.id) - sessionNumber(left.id))[0] ?? null
}

async function createSession(client: LocalIpcClient, workspace: string, worktree: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(createSessionRequest(workspace, worktree))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionCreated")
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

async function getSessionState(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
  return payload.session
}

async function getSessionHistory(client: LocalIpcClient, sessionId: string): Promise<SessionHistoryEntry[]> {
  const response = await client.send<Record<string, unknown>>(getSessionHistoryRequest(sessionId))
  const payload = expectVariant<{ entries: SessionHistoryEntry[] }>(response, "SessionHistory")
  return payload.entries
}

async function launchProviderRun(
  client: LocalIpcClient,
  sessionId: string,
  accountProfile: string,
  model: string,
): Promise<void> {
  await client.send<Record<string, unknown>>(launchProviderRunRequest(sessionId, accountProfile, model))
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

function createSessionRequest(workspaceId: string, worktreeId: string) {
  return {
    CreateSession: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
    },
  }
}

function listSessionsRequest() {
  return { ListSessions: null }
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

function getSessionStateRequest(sessionId: string) {
  return {
    GetSessionState: {
      session_id: sessionId,
    },
  }
}

function getSessionHistoryRequest(sessionId: string) {
  return {
    GetSessionHistory: {
      session_id: sessionId,
      limit: BOOTSTRAP_HISTORY_LIMIT,
      max_chars: BOOTSTRAP_HISTORY_MAX_CHARS,
    },
  }
}

function launchProviderRunRequest(sessionId: string, accountProfile: string, model: string) {
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: accountProfile,
      model,
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

function sessionNumber(sessionId: string): number {
  return Number.parseInt(sessionId.replace(/^session-/, ""), 10) || 0
}

function trimSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text
}

function formatError(error: unknown): string {
  return describeCliError(error)
}

function printUsage() {
  process.stdout.write(
    "usage: arroba-cli [--socket PATH] [--session ID] [--client-id ID] [--model MODEL] [--account-profile PROFILE] [--workspace PATH] [--worktree PATH]\n       arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]\n\ncommands:\n  /stop   request cancellation of the active provider turn\n  /exit   exit the CLI\n",
  )
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
