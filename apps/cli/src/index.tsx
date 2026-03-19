import path from "node:path"
import process from "node:process"
import { homedir } from "node:os"
import { setTimeout as sleep } from "node:timers/promises"

import { BoxRenderable, ScrollBoxRenderable, TextAttributes, TextRenderable, type KeyBinding, type TextareaRenderable } from "@opentui/core"
import { render, useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/solid"
import { createMemo, createSignal, onCleanup, onMount } from "solid-js"
import { createStore, produce } from "solid-js/store"

import { LocalIpcClient } from "./ipc.js"
import { createProcessLogger, type ArrobaLogger } from "./logging.js"
import { runLogViewer } from "./logs.js"
import {
  DEFAULT_CONNECTED_STATUS,
  describeCliError,
  getExitCleanupDecision,
  getPollRecoveryDecision,
} from "./runtime.js"
import { EmptyBorder, PromptBorderChars, SplitBorder, theme } from "./theme.js"

const PROMPT_KEYBINDINGS = [
  { name: "return", action: "submit" },
  { name: "return", meta: true, action: "newline" },
] satisfies KeyBinding[]

type RuntimeSession = {
  id: string
  workspace_id: string
  worktree_id: string
  status: string
  active_provider_run_id: string | null
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
  kind: "provider_output" | "prompt_echo"
  bytes: number[]
}

type PromptSubmittedPayload = {
  outcome: Record<string, unknown>
  session: RuntimeSession
}

type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "notice"
  text: string
  emphasis?: "muted" | "warning" | "error"
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
  options: CliOptions
}

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
  getLogger("cli.main")?.debug("sent initial resize", {
    session_id: bootstrap.session.id,
  })

  render(
    () => <ArrobaCliApp bootstrap={bootstrap} />, 
    {
      targetFps: 60,
      gatherStats: false,
      exitOnCtrlC: false,
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
  const [sessionState, setSessionState] = createSignal(session)
  const [entries, setEntries] = createStore<TranscriptEntry[]>([])
  const [statusLine, setStatusLine] = createSignal(DEFAULT_CONNECTED_STATUS)
  const [fatalError, setFatalError] = createSignal<string | null>(null)
  const [submitting, setSubmitting] = createSignal(false)
  const [entryCounter, setEntryCounter] = createSignal(0)
  let promptInput: TextareaRenderable | undefined
  let transcriptScrollbox: ScrollBoxRenderable | undefined
  let closing = false
  let exitCleanupFailed = false
  const degradedPollers = new Set<string>()

  const queueDepth = createMemo(() => sessionState().queued_prompts.length)
  const activePrompt = createMemo(() => sessionState().active_prompt)
  const footerHint = createMemo(() => {
    if (fatalError()) {
      return fatalError()!
    }
    if (activePrompt()) {
      return queueDepth() > 0
        ? `Processing ${activePrompt()!.id}; ${queueDepth()} queued.`
        : `Processing ${activePrompt()!.id}.`
    }
    return statusLine()
  })

  const appendEntry = (entry: Omit<TranscriptEntry, "id">) => {
    const nextId = entryCounter() + 1
    setEntryCounter(nextId)
    setEntries(entries.length, { id: nextId, ...entry })
    renderTranscript()
    appLogger?.debug("appended transcript entry", {
      role: entry.role,
      entry_id: nextId,
      total_entries: entries.length + 1,
      chars: entry.text.length,
    })
  }

  const appendUserPrompt = (text: string) => {
    appendEntry({ role: "user", text: trimSingleTrailingNewline(text) })
    setSubmitting(true)
  }

  const appendNotice = (text: string, emphasis: TranscriptEntry["emphasis"] = "muted") => {
    appendEntry({ role: "notice", text, emphasis })
  }

  const appendAssistantChunk = (chunk: string) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }
    setSubmitting(false)
    let merged = false
    setEntries(
      produce((draft) => {
        const last = draft.at(-1)
        if (last?.role === "assistant") {
          last.text += normalized
          merged = true
          return
        }
        draft.push({
          id: entryCounter() + 1,
          role: "assistant",
          text: normalized,
        })
      }),
    )
    renderTranscript()
    if (merged) {
      appLogger?.debug("merged assistant chunk", {
        chars: normalized.length,
        total_entries: entries.length,
      })
      return
    }
    setEntryCounter((value) => value + 1)
    appLogger?.debug("created assistant transcript entry", {
      entry_id: entryCounter() + 1,
      chars: normalized.length,
      total_entries: entries.length + 1,
    })
  }

  const renderTranscript = () => {
    if (!transcriptScrollbox) {
      return
    }

    for (const child of [...transcriptScrollbox.getChildren()]) {
      transcriptScrollbox.remove(child.id)
    }

    if (entries.length === 0) {
      transcriptScrollbox.add(buildEmptyTranscriptRenderable(renderer))
    } else {
      for (const entry of entries) {
        transcriptScrollbox.add(buildTranscriptEntryRenderable(renderer, entry))
      }
    }

    transcriptScrollbox.requestRender()
  }

  const requestExit = async () => {
    if (closing && exitCleanupFailed) {
      appLogger?.warn("forcing cli exit after prior cleanup failure")
      process.exit(1)
    }
    if (closing) {
      return
    }
    closing = true
    appLogger?.info("requested cli exit", {
      created_session: createdSession,
    })
    try {
      if (createdSession) {
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
        process.exit(decision.exitCode)
      }
      return
    }
    appLogger?.info("cli exit cleanup completed")
    process.exit(0)
  }

  const submitPrompt = async () => {
    if (!promptInput) {
      return
    }

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
      } catch (error) {
        appLogger?.error("active prompt cancellation failed", {
          error: formatError(error),
        })
        setFatalError(formatError(error))
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
      setSessionState(payload.session)
      appendUserPrompt(prompt)
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
      promptInput.clear()
    } catch (error) {
      appLogger?.error("prompt submission failed", {
        error: formatError(error),
      })
      setFatalError(formatError(error))
    }
  }

  useKeyboard((event) => {
    if (event.ctrl && event.name === "c") {
      event.preventDefault()
      void requestExit()
    }
  })

  onMount(() => {
    promptInput?.focus()
    renderTranscript()

    const onResize = () => {
      void maybeResize(client, sessionState().id)
    }
    process.stdout.on("resize", onResize)

    const markPollerDegraded = (operation: string, message: string) => {
      const wasHealthy = degradedPollers.size === 0
      degradedPollers.add(operation)
      appLogger?.warn("poller entered degraded mode", {
        operation,
        degraded_pollers: [...degradedPollers],
      })
      setStatusLine(message)
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
        setStatusLine(DEFAULT_CONNECTED_STATUS)
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
          setFatalError(formatError(error))
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
        if (payload.records.length > 0) {
          appLogger?.debug("received terminal output records", {
            record_count: payload.records.length,
            kinds: payload.records.map((record) => record.kind),
          })
        }
        for (const record of payload.records) {
          const text = Buffer.from(record.bytes).toString("utf8")
          if (record.kind === "prompt_echo") {
            appendEntry({ role: "user", text: trimSingleTrailingNewline(text) })
          } else {
            appendAssistantChunk(text)
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
        if (payload.notices.length > 0) {
          appLogger?.debug("received runtime notices", {
            notice_count: payload.notices.length,
          })
        }
        for (const notice of payload.notices) {
          appendNotice(notice.message)
        }
      })
    }

    const pollSessionState = async () => {
      await runPollingLoop("polling session state", 250, async () => {
        const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionState().id))
        const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
        setSessionState(payload.session)
        if (!payload.session.active_prompt) {
          setSubmitting(false)
        }
      })
    }

    void pollOutput()
    void pollNotices()
    void pollSessionState()

    onCleanup(() => {
      closing = true
      process.stdout.off("resize", onResize)
    })
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
          <box flexDirection="column" alignItems="flex-end">
            <text fg={activePrompt() ? theme.primary : theme.success}>
              {activePrompt() ? "ACTIVE" : "IDLE"}
            </text>
            <text fg={theme.textMuted}>
              {sessionState().active_provider_run_id ?? "starting provider"}
            </text>
          </box>
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
            <text fg={fatalError() ? theme.error : submitting() ? theme.primary : theme.textMuted}>
              {fatalError() ? "error" : submitting() ? "thinking" : footerHint()}
            </text>
          </box>
          <textarea
            ref={(value) => {
              promptInput = value
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
        <text fg={theme.textMuted}>
          Session {sessionState().id} • {queueDepth()} queued • Enter sends • Meta+Enter adds newline • Ctrl+C or /exit to leave
        </text>
      </box>
    </box>
  )
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
  const accent = transcriptAccent(entry)
  const bodyColor = entry.role === "assistant" ? theme.backgroundPanel : theme.backgroundElement

  wrapper.add(
    new TextRenderable(renderer, {
      content: transcriptLabel(entry),
      fg: theme.textMuted,
    }),
  )

  const border = new BoxRenderable(renderer, {
    border: ["left"],
    customBorderChars: SplitBorder.customBorderChars,
    borderColor: accent,
  })
  const body = new BoxRenderable(renderer, {
    paddingLeft: 2,
    paddingRight: 1,
    paddingTop: 0,
    paddingBottom: 0,
    backgroundColor: bodyColor,
  })
  body.add(
    new TextRenderable(renderer, {
      content: entry.text,
      fg: transcriptTextColor(entry),
      wrapMode: "word",
    }),
  )
  border.add(body)
  wrapper.add(border)
  return wrapper
}

function buildEmptyTranscriptRenderable(renderer: ReturnType<typeof useRenderer>) {
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 1,
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
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? theme.error
      : entry.emphasis === "warning"
        ? theme.warning
        : theme.textMuted
  }
  return theme.borderSubtle
}

function transcriptLabel(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return "Prompt"
  }
  if (entry.role === "notice") {
    return "Notice"
  }
  return "Model"
}

function transcriptTextColor(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return theme.primary
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

  return {
    client,
    session: await getSessionState(client, session.id),
    attachment,
    createdSession,
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
