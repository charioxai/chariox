import { execFile, spawn } from "node:child_process"
import { appendFileSync } from "node:fs"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import { createServer, type IncomingMessage, type ServerResponse } from "node:http"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import {
  normalizeRuntimeSession,
  type AgentInstance,
  type PromptAttachmentPart,
  type PromptQueueItem,
  type RuntimeAttachment,
  type RuntimeProviderRun,
  type RuntimeSession,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  aliasAgentRequest,
  appendNativeProviderOutputRequest,
  attachToSessionRequest,
  completePromptRequest,
  createSessionRequest,
  getProviderRunRequest,
  getSessionStateRequest,
  launchProviderRunRequest,
  pumpTerminalOutputRequest,
  requestNativeProviderInteractionRequest,
  resolveSessionRequest,
  resizeTerminalRequest,
  sendTerminalInputRequest,
  spawnAgentRequest,
  submitPromptRequest,
} from "../ipc-requests.js"
import { localAttachmentPath, preparePromptAttachmentsForSubmit, promptAttachmentTransferIsForced } from "../prompt-attachment-transfer.js"
import { classifyPromptAttachment } from "../prompt-attachments.js"
import { hiddenInstructionsEnd, hiddenInstructionsStart, redactHiddenInstructions } from "./hidden-instructions.js"

const CLAUDE_ATTACHMENT_CONTEXT_BYTES = 200_000

const execFileAsync = promisify(execFile)

type NativeClaudeOptions = {
  sessionRef?: string
  socketPath?: string
  kernelUrl?: string
  kernelPort?: string
  relayUrl?: string
  relayToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
  clientId: string
  workspace?: string
  worktree?: string
  alias?: string
  agentAlias?: string
  model: string
  effort: string
  mode: "build" | "plan"
  permissions: "required" | "yolo"
  initialPrompt?: string
  detachedScreen: boolean
  remoteRendered: boolean
}

type ClaudeHookEvent = {
  index: number
  hook_event_name: string
  prompt?: string | null
  transcript_path?: string | null
  permission_mode?: string | null
  tool_name?: string | null
  tool_input?: unknown
  tool_response?: unknown
  error?: unknown
}

type ClaudeTuiController = {
  label: string
  submitPrompt: (prompt: string) => Promise<void>
  waitForExit: () => Promise<void>
  stop: () => Promise<void>
}

export async function runClaudeNativeTui(args: string[]): Promise<void> {
  const options = parseNativeClaudeArgs(args)
  const inferredTargets = await inferWorkspaceTargetsFromLaunchDirectory(process.cwd())
  const workspace = options.workspace ?? inferredTargets.workspace
  const worktree = options.worktree ?? inferredTargets.worktree
  const kernelEndpoint = options.relayUrl ?? options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint(options.kernelPort)
  const client = new LocalIpcClient(kernelEndpoint, options.relayUrl
    ? {
      relayAuthToken: options.relayToken,
      targetDaemonId: options.targetDaemonId,
      targetDaemonAlias: options.targetDaemonAlias,
    }
    : undefined)
  if (options.remoteRendered) {
    await runClaudeRemoteRendered(options, client, workspace, worktree)
    return
  }

  const tempRoot = path.join(os.tmpdir(), `arroba-claude-native-${process.pid}-${Date.now()}`)
  const eventsFile = path.join(tempRoot, "events.jsonl")
  const contextFile = path.join(tempRoot, "hidden-context.txt")
  const attachmentContextDir = path.join(tempRoot, "attachments")
  const settingsPath = path.join(tempRoot, "settings.json")
  const hookHandlerPath = path.join(tempRoot, "hook-handler.mjs")
  const screenName = `arroba-claude-${process.pid}-${Date.now()}`
  const screenLogDir = path.join(tempRoot, "screen")
  let bridge: { stop: () => void } | null = null
  let permissionBridge: { url: string; stop: () => Promise<void> } | null = null
  let pump: { stop: () => void } | null = null
  let tui: ClaudeTuiController | null = null

  try {
    await mkdir(screenLogDir, { recursive: true })
    await writeFile(contextFile, "", "utf8")
    await writeClaudeHookHandler(hookHandlerPath)
    await writeFile(settingsPath, JSON.stringify(claudeHookSettings(hookHandlerPath), null, 2), "utf8")

    const created = options.sessionRef
      ? null
      : await createSession(client, workspace, worktree, options.alias, options.model, options.effort, options.mode, options.permissions)
    const session = created?.session ?? await resolveSession(client, options.sessionRef!, workspace)
    const attachment = await attachToSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await maybeAliasAgent(client, session.id, created.agent, options.agentAlias)
      : await spawnClaudeAgent(client, session.id, options.agentAlias, options.model, options.effort, worktree, options.mode, options.permissions)
    const launched = await launchClaudeNativeRun(client, session.id, agent.id, options.model, options.effort)
    const run = await waitForProviderRunReady(client, launched.id)
    permissionBridge = await startClaudePermissionBridge({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
    })

    process.stderr.write([
      "[arroba claude native-tui]",
      `  arroba session: ${session.id}${session.alias ? ` (${session.alias})` : ""}`,
      `  arroba agent:   ${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`,
      `  provider run:   ${run.id}`,
      `  tui:            ${options.detachedScreen ? `screen:${screenName}` : "attached-pty"}`,
      ...(options.detachedScreen ? [`  screen:         ${screenName}`] : []),
      "  prompt policy:  Claude Code TUI is native; Arroba observes hooks and injects queued prompts through the PTY",
      "",
    ].join("\n"))

    const launchOptions: Parameters<typeof startClaudeScreen>[2] = {
      worktree,
      settingsPath,
      model: options.model,
      effort: options.effort,
      permissions: options.permissions,
      env: {
        ...process.env,
        ARROBA_CLAUDE_NATIVE_EVENTS: eventsFile,
        ARROBA_CLAUDE_NATIVE_CONTEXT: contextFile,
        ARROBA_CLAUDE_NATIVE_HOOK_BRIDGE_URL: permissionBridge.url,
      },
    }
    tui = options.detachedScreen
      ? await startClaudeScreen(screenName, screenLogDir, launchOptions)
      : await startClaudeAttachedPty(launchOptions)
    bridge = startClaudeBridge({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      providerRunId: run.id,
      eventsFile,
      contextFile,
      attachmentContextDir,
      worktree,
      inlineLocalAttachments: Boolean(options.relayUrl) || promptAttachmentTransferIsForced(),
      submitPrompt: tui.submitPrompt,
    })
    pump = startKernelPumpLoop(client, session.id, attachment.id)
    if (options.initialPrompt) {
      await sleep(1_000)
      await tui.submitPrompt(options.initialPrompt)
    }

    await tui.waitForExit()
  } finally {
    bridge?.stop()
    pump?.stop()
    await tui?.stop()
    await permissionBridge?.stop().catch(() => {})
    await client.close()
    if (!options.detachedScreen) {
      await rm(tempRoot, { recursive: true, force: true }).catch(() => {})
    }
  }
}

function parseNativeClaudeArgs(args: string[]): NativeClaudeOptions {
  const options: NativeClaudeOptions = {
    clientId: `arroba-claude-native-${process.pid}`,
    model: "sonnet",
    effort: "low",
    mode: "build",
    permissions: "required",
    detachedScreen: false,
    remoteRendered: false,
  }
  const positional: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] ?? ""
    const next = () => {
      const value = args[index + 1]
      if (!value) throw new Error(`missing value for ${arg}`)
      index += 1
      return value
    }
    switch (arg) {
      case "--socket":
        options.socketPath = next()
        break
      case "--kernel-url":
        options.kernelUrl = next()
        break
      case "--kernel-port":
      case "--port":
        options.kernelPort = parseKernelPort(next(), arg)
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
      case "--client-id":
        options.clientId = next()
        break
      case "--workspace":
        options.workspace = path.resolve(next())
        break
      case "--worktree":
        options.worktree = path.resolve(next())
        break
      case "--alias":
        options.alias = next()
        break
      case "--agent-alias":
        options.agentAlias = next()
        break
      case "--model":
      case "-m":
        options.model = next()
        break
      case "--effort":
        options.effort = next()
        break
      case "--mode":
        options.mode = parseMode(next())
        break
      case "--permissions":
        options.permissions = parsePermissions(next())
        break
      case "--initial-prompt":
        options.initialPrompt = next()
        break
      case "--detached-screen":
        options.detachedScreen = true
        break
      case "--remote-rendered":
        options.remoteRendered = true
        break
      case "--help":
      case "-h":
        printNativeClaudeUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) throw new Error(`unknown claude argument ${arg}`)
        positional.push(arg)
    }
  }
  if (positional.length > 1) throw new Error(`unexpected claude arguments: ${positional.slice(1).join(" ")}`)
  if (positional[0] !== undefined) options.sessionRef = positional[0]
  return options
}

function printNativeClaudeUsage() {
  process.stdout.write([
    "Usage: arroba claude [session_id] [options]",
    "",
    "Launch Claude Code's native TUI as an Arroba-managed native provider agent.",
    "",
    "Options:",
    "  --kernel-port, --port <port>     Kernel websocket port (default 43119)",
    "  --kernel-url <url>               Kernel websocket URL",
    "  --workspace <path>               Workspace root",
    "  --worktree <path>                Worktree root",
    "  --alias <name>                   Alias for a newly-created session",
    "  --agent-alias <name>             Alias for the Claude native agent",
    "  --model <model>                  Claude model argument (default sonnet)",
    "  --effort <effort>                Claude effort argument (default low)",
    "  --mode <build|plan>              Arroba agent mode (default build)",
    "  --permissions <required|yolo>    Claude permission mode mapping (default required)",
    "  --remote-rendered                Run Claude Code in the target kernel PTY and render it here",
    "",
  ].join("\n"))
}

async function runClaudeRemoteRendered(
  options: NativeClaudeOptions,
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
): Promise<void> {
  let pump: { stop: () => void } | null = null
  let disposeEvents: (() => void) | null = null
  let restoreStdin: (() => void) | null = null
  try {
    const created = options.sessionRef
      ? null
      : await createSession(client, workspace, worktree, options.alias, options.model, options.effort, options.mode, options.permissions)
    const session = created?.session ?? await resolveSession(client, options.sessionRef!, workspace)
    const attachment = await attachToSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await maybeAliasAgent(client, session.id, created.agent, options.agentAlias)
      : await spawnClaudeAgent(client, session.id, options.agentAlias, options.model, options.effort, worktree, options.mode, options.permissions)
    const launched = await launchClaudeRemoteRenderedRun(client, session.id, agent.id, options.model, options.effort)
    const run = await waitForProviderRunReady(client, launched.id)

    process.stderr.write([
      "[arroba claude remote-native-tui]",
      `  arroba session: ${session.id}${session.alias ? ` (${session.alias})` : ""}`,
      `  arroba agent:   ${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`,
      `  provider run:   ${run.id}`,
      "  tui:            target-kernel-pty",
      "",
    ].join("\n"))

    disposeEvents = client.onKernelEvent((event) => {
      if (event.event !== "terminal_output") return
      writeRemoteRenderedTerminalRecords(event.records, run.id)
    })
    await client.subscribeToKernelEvents(session.id, attachment.id)
    pump = startRemoteRenderedPumpLoop(client, session.id, attachment.id, run.id)
    restoreStdin = forwardStdinToProviderRun(client, session.id, attachment.id, run.id)
    installResizeForwarder(client, session.id)
    if (options.initialPrompt) {
      await sleep(2_000)
      await client.send<Record<string, unknown>>(
        sendTerminalInputRequest(session.id, attachment.id, options.initialPrompt, run.id),
      )
      await sleep(500)
      await client.send<Record<string, unknown>>(
        sendTerminalInputRequest(session.id, attachment.id, "\r", run.id),
      )
    }
    await waitForRemoteRenderedRunExit(client, run.id)
  } finally {
    restoreStdin?.()
    disposeEvents?.()
    pump?.stop()
    await client.unsubscribeFromKernelEvents().catch(() => {})
    await client.close()
  }
}

function forwardStdinToProviderRun(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
): () => void {
  const wasRaw = Boolean(process.stdin.isTTY && process.stdin.isRaw)
  const onData = (chunk: Buffer) => {
    void client.send<Record<string, unknown>>(
      sendTerminalInputRequest(sessionId, attachmentId, chunk, providerRunId),
    ).catch(() => {})
  }
  if (process.stdin.isTTY) process.stdin.setRawMode?.(true)
  process.stdin.resume()
  process.stdin.on("data", onData)
  return () => {
    process.stdin.off("data", onData)
    if (process.stdin.isTTY) process.stdin.setRawMode?.(wasRaw)
  }
}

function installResizeForwarder(client: LocalIpcClient, sessionId: string) {
  const sendResize = () => {
    const cols = process.stdout.columns
    const rows = process.stdout.rows
    if (!cols || !rows) return
    void client.send<Record<string, unknown>>(resizeTerminalRequest(sessionId, cols, rows)).catch(() => {})
  }
  process.stdout.on("resize", sendResize)
  sendResize()
}

function startClaudeBridge(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  providerRunId: string
  eventsFile: string
  contextFile: string
  attachmentContextDir: string
  worktree: string
  inlineLocalAttachments: boolean
  submitPrompt: (prompt: string) => Promise<void>
}): { stop: () => void } {
  let stopped = false
  let nextEventIndex = 0
  let activePromptId: string | null = null
  const injectedPromptIds = new Set<string>()
  const nativeSubmittedPromptIds = new Set<string>()
  const transcriptLineOffsets = new Map<string, number>()

  const loop = async () => {
    while (!stopped) {
      try {
        const events = await readClaudeHookEvents(options.eventsFile)
        for (const event of events.slice(nextEventIndex)) {
          nextEventIndex = Math.max(nextEventIndex, event.index + 1)
          if (event.hook_event_name === "UserPromptSubmit" && event.prompt) {
            const prompt = event.prompt.trim()
            const isInjected = activePromptId && injectedPromptIds.has(activePromptId)
            if (!isInjected) {
              const attachments = await preparePromptAttachmentsForSubmit(
                extractClaudeNativePromptAttachments(prompt, options.worktree),
                { inlineLocalFiles: options.inlineLocalAttachments },
              )
              if (attachments.length > 0) {
                debugNativeClaude("native_prompt_attachments_observed", {
                  attachmentCount: attachments.length,
                })
              }
              const response = await options.client.send<Record<string, unknown>>(
                submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
              )
              const submittedPrompt = extractSubmittedPromptId(response, options.agentId)
              if (submittedPrompt) {
                activePromptId = submittedPrompt
                nativeSubmittedPromptIds.add(submittedPrompt)
              } else {
                const state = await sessionState(options.client, options.sessionId)
                activePromptId = promptForAgent(state, options.agentId)?.id ?? activePromptId
                if (activePromptId) nativeSubmittedPromptIds.add(activePromptId)
              }
            }
          } else if (event.hook_event_name === "Stop") {
            const output = event.transcript_path
              ? await waitForAssistantText(event.transcript_path, transcriptLineOffsets)
              : ""
            if (output.trim()) {
              await options.client.send<Record<string, unknown>>(
                appendNativeProviderOutputRequest(
                  options.sessionId,
                  options.attachmentId,
                  options.providerRunId,
                  "provider_output",
                  output.endsWith("\n") ? output : `${output}\n`,
                  `claude-native-${Date.now()}`,
                ),
              )
            }
            await options.client.send<Record<string, unknown>>(completePromptRequest(options.sessionId))
              .catch(() => ({}))
            activePromptId = null
            await writeFile(options.contextFile, "", "utf8").catch(() => {})
          }
        }

        const state = await sessionState(options.client, options.sessionId)
        const activePrompt = promptForAgent(state, options.agentId)
        if (activePrompt && activePrompt.id !== activePromptId && !nativeSubmittedPromptIds.has(activePrompt.id)) {
          activePromptId = activePrompt.id
          injectedPromptIds.add(activePrompt.id)
          const hidden = extractHiddenInstructions(activePrompt.prompt)
          const attachmentContext = await formatClaudeAttachmentContext(
            activePrompt.attachments ?? [],
            options.attachmentContextDir,
          )
          await writeFile(options.contextFile, joinClaudeAdditionalContext(hidden, attachmentContext), "utf8")
          if ((activePrompt.attachments?.length ?? 0) > 0) {
            debugNativeClaude("attachments_forwarded", {
              promptId: activePrompt.id,
              attachmentCount: activePrompt.attachments?.length ?? 0,
            })
          }
          const visible = redactHiddenInstructions(activePrompt.prompt).trim()
          if (visible) {
            await options.submitPrompt(visible)
          }
        }
      } catch (error) {
        process.stderr.write(`[arroba claude native-tui] bridge warning: ${error instanceof Error ? error.message : String(error)}\n`)
      }
      await sleep(500)
    }
  }
  void loop()
  return {
    stop: () => {
      stopped = true
    },
  }
}

async function startClaudePermissionBridge(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
}): Promise<{ url: string; stop: () => Promise<void> }> {
  const server = createServer((request, response) => {
    void handleClaudePermissionBridgeRequest(options, request, response)
  })
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject)
      resolve()
    })
  })
  const address = server.address()
  if (!address || typeof address === "string") {
    await new Promise<void>((resolve) => server.close(() => resolve()))
    throw new Error("failed to start Claude permission bridge")
  }
  return {
    url: `http://127.0.0.1:${address.port}`,
    stop: () => new Promise((resolve) => server.close(() => resolve())),
  }
}

async function handleClaudePermissionBridgeRequest(
  options: {
    client: LocalIpcClient
    sessionId: string
    attachmentId: string
    agentId: string
  },
  request: IncomingMessage,
  response: ServerResponse,
) {
  if (request.method !== "POST" || request.url !== "/permission") {
    writeJsonResponse(response, 404, { error: "not found" })
    return
  }
  try {
    const payload = await readJsonRequest(request)
    if (!shouldBridgeClaudePermission(payload)) {
      writeJsonResponse(response, 200, { handled: false })
      return
    }
    if (!await shouldBridgeCurrentClaudePermission(
      options.client,
      options.sessionId,
      options.agentId,
      options.attachmentId,
    )) {
      writeJsonResponse(response, 200, { handled: false })
      return
    }
    const toolName = typeof payload.tool_name === "string" ? payload.tool_name : "tool"
    const interactionId = `claude-native-permission-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`
    const interactionResponse = await options.client.send<Record<string, unknown>>(
      requestNativeProviderInteractionRequest(
        options.sessionId,
        options.agentId,
        interactionId,
        `Approve Claude Code ${toolName}?`,
        formatClaudePermissionMessage(payload),
        300,
      ),
    )
    const resolution = expectVariant<{ resolution: { status?: string; choice_id?: string | null; reply?: string | null } }>(
      interactionResponse,
      "NativeProviderInteractionResolved",
    ).resolution
    const allowed = resolution.reply === "allow" || resolution.choice_id === "allow_once"
    writeJsonResponse(response, 200, {
      handled: true,
      permissionDecision: allowed ? "allow" : "deny",
      permissionDecisionReason: allowed
        ? "Approved through Arroba."
        : resolution.status === "timed_out"
          ? "Timed out waiting for Arroba approval."
          : "Denied through Arroba.",
    })
  } catch (error) {
    writeJsonResponse(response, 500, {
      error: error instanceof Error ? error.message : String(error),
    })
  }
}

async function shouldBridgeCurrentClaudePermission(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  nativeAttachmentId: string,
): Promise<boolean> {
  const session = await sessionState(client, sessionId)
  const activePrompt = promptForAgent(session, agentId)
  return Boolean(activePrompt && activePrompt.source_attachment_id !== nativeAttachmentId)
}

type ClaudePermissionPayload = {
  hook_event_name?: unknown
  permission_mode?: unknown
  tool_name?: unknown
  tool_input?: unknown
  prompt?: unknown
}

function shouldBridgeClaudePermission(payload: ClaudePermissionPayload): boolean {
  if (payload.hook_event_name !== "PreToolUse" && payload.hook_event_name !== "PermissionRequest") return false
  const toolName = typeof payload.tool_name === "string" ? payload.tool_name : ""
  return new Set(["Bash", "Write", "Edit", "MultiEdit", "NotebookEdit"]).has(toolName)
}

function formatClaudePermissionMessage(payload: ClaudePermissionPayload): string {
  const toolName = typeof payload.tool_name === "string" ? payload.tool_name : "tool"
  const details = formatClaudeToolInput(payload.tool_input)
  const permissionMode = typeof payload.permission_mode === "string" ? payload.permission_mode : null
  return [
    `Claude Code wants to run ${toolName}.`,
    ...(permissionMode ? [`Permission mode: ${permissionMode}.`] : []),
    ...(details ? ["", details] : []),
  ].join("\n")
}

function formatClaudeToolInput(input: unknown): string {
  if (!input || typeof input !== "object") return ""
  const record = input as Record<string, unknown>
  if (typeof record.command === "string") return ["Command:", "", record.command].join("\n")
  if (typeof record.file_path === "string") {
    const pieces = [`File: ${record.file_path}`]
    if (typeof record.old_string === "string") pieces.push("", "Old:", record.old_string)
    if (typeof record.new_string === "string") pieces.push("", "New:", record.new_string)
    if (typeof record.content === "string") pieces.push("", "Content:", record.content)
    return pieces.join("\n")
  }
  try {
    return JSON.stringify(input, null, 2)
  } catch {
    return String(input)
  }
}

async function readJsonRequest(request: IncomingMessage): Promise<ClaudePermissionPayload> {
  const chunks: Buffer[] = []
  for await (const chunk of request) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
  const raw = Buffer.concat(chunks).toString("utf8")
  return raw.trim() ? JSON.parse(raw) as ClaudePermissionPayload : {}
}

function writeJsonResponse(response: ServerResponse, statusCode: number, body: Record<string, unknown>) {
  response.writeHead(statusCode, { "content-type": "application/json" })
  response.end(JSON.stringify(body))
}

async function writeClaudeHookHandler(file: string) {
  await writeFile(file, `#!/usr/bin/env node
import { appendFileSync, readFileSync } from "node:fs"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const raw = Buffer.concat(chunks).toString("utf8")
let input = {}
try {
  input = raw.trim() ? JSON.parse(raw) : {}
} catch (error) {
  input = { hook_event_name: "parse_error", raw, error: String(error) }
}
const eventName = input.hook_event_name ?? "unknown"
appendFileSync(process.env.ARROBA_CLAUDE_NATIVE_EVENTS, JSON.stringify({
  at: new Date().toISOString(),
  hook_event_name: eventName,
  prompt: input.prompt ?? null,
  transcript_path: input.transcript_path ?? null,
  permission_mode: input.permission_mode ?? null,
  tool_name: input.tool_name ?? null,
  tool_input: input.tool_input ?? null,
  tool_response: input.tool_response ?? null,
  error: input.error ?? null,
}) + "\\n")

if (eventName === "UserPromptSubmit") {
  let additionalContext = ""
  try {
    additionalContext = readFileSync(process.env.ARROBA_CLAUDE_NATIVE_CONTEXT, "utf8")
  } catch {}
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext
    }
  }))
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  const bridgeUrl = process.env.ARROBA_CLAUDE_NATIVE_HOOK_BRIDGE_URL
  if (bridgeUrl) {
    try {
      const response = await fetch(new URL("/permission", bridgeUrl), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input)
      })
      if (response.ok) {
        const decision = await response.json()
        if (decision?.handled && decision.permissionDecision) {
          process.stdout.write(JSON.stringify({
            hookSpecificOutput: {
              hookEventName: eventName,
              permissionDecision: decision.permissionDecision,
              permissionDecisionReason: decision.permissionDecisionReason ?? "Resolved through Arroba."
            }
          }))
        }
      }
    } catch {}
  }
}
`, "utf8")
}

function claudeHookSettings(handlerPath: string) {
  const command = `node ${JSON.stringify(handlerPath)}`
  return {
    hooks: {
      UserPromptSubmit: [{ hooks: [{ type: "command", command }] }],
      Stop: [{ hooks: [{ type: "command", command }] }],
      StopFailure: [{ hooks: [{ type: "command", command }] }],
      SessionEnd: [{ hooks: [{ type: "command", command }] }],
      PermissionRequest: [{ matcher: "*", hooks: [{ type: "command", command }] }],
      PreToolUse: [{ matcher: "*", hooks: [{ type: "command", command }] }],
      PostToolUse: [{ matcher: "*", hooks: [{ type: "command", command }] }],
    },
  }
}

async function startClaudeScreen(name: string, logDir: string, options: {
  worktree: string
  settingsPath: string
  model: string
  effort: string
  permissions: "required" | "yolo"
  env: NodeJS.ProcessEnv
}): Promise<ClaudeTuiController> {
  const claudeArgs = claudeCommandArgs(options)
  await execFileAsync("screen", [
    "-dmS",
    name,
    "-L",
    "bash",
    "-lc",
    `cd ${shellQuote(options.worktree)} && exec ${claudeArgs.map(shellQuote).join(" ")}`,
  ], {
    cwd: logDir,
    env: options.env,
  })
  return {
    label: `screen:${name}`,
    submitPrompt: async (prompt) => {
      await waitForScreenReady(logDir, name)
      await submitScreenPrompt(name, prompt)
    },
    waitForExit: async () => {
      while (await screenExists(name)) await sleep(500)
    },
    stop: () => screenQuit(name),
  }
}

async function startClaudeAttachedPty(options: {
  worktree: string
  settingsPath: string
  model: string
  effort: string
  permissions: "required" | "yolo"
  env: NodeJS.ProcessEnv
}): Promise<ClaudeTuiController> {
  const command = `cd ${shellQuote(options.worktree)} && exec ${claudeCommandArgs(options).map(shellQuote).join(" ")}`
  const child = spawn("script", scriptArgs(command), {
    cwd: options.worktree,
    env: options.env,
    stdio: ["pipe", "pipe", "pipe"],
  })
  child.stdout?.on("data", (chunk) => process.stdout.write(chunk))
  child.stderr?.on("data", (chunk) => process.stderr.write(chunk))

  const stdin = child.stdin
  if (!stdin) {
    child.kill("SIGTERM")
    throw new Error("failed to start attached Claude PTY: script stdin was unavailable")
  }
  const ready = sleep(1_500)

  const forwardInput = (chunk: Buffer) => {
    if (!stdin.destroyed) stdin.write(chunk)
  }
  const wasRaw = Boolean(process.stdin.isTTY && process.stdin.isRaw)
  if (process.stdin.isTTY) process.stdin.setRawMode?.(true)
  process.stdin.resume()
  process.stdin.on("data", forwardInput)

  let stopped = false
  const waitForExit = new Promise<void>((resolve, reject) => {
    child.once("error", (error) => reject(new Error(`failed to start attached Claude PTY via script: ${error.message}`)))
    child.once("exit", () => resolve())
  }).finally(() => {
    process.stdin.off("data", forwardInput)
    if (process.stdin.isTTY) process.stdin.setRawMode?.(wasRaw)
  })

  return {
    label: "attached-pty",
    submitPrompt: async (prompt) => {
      await ready
      if (stdin.destroyed) throw new Error("attached Claude PTY is closed")
      stdin.write(prompt)
      await sleep(250)
      stdin.write("\r")
    },
    waitForExit: () => waitForExit,
    stop: async () => {
      if (stopped) return
      stopped = true
      if (child.exitCode == null && child.signalCode == null) {
        child.kill("SIGTERM")
        await Promise.race([waitForExit, sleep(2_000)]).catch(() => {})
        if (child.exitCode == null && child.signalCode == null) child.kill("SIGKILL")
      }
    },
  }
}

function claudeCommandArgs(options: {
  settingsPath: string
  model: string
  effort: string
  permissions: "required" | "yolo"
}): string[] {
  return [
    "claude",
    "--settings",
    options.settingsPath,
    "--permission-mode",
    options.permissions === "yolo" ? "bypassPermissions" : "default",
    "--model",
    options.model,
    "--effort",
    options.effort,
  ]
}

function scriptArgs(command: string): string[] {
  if (process.platform === "linux") return ["-q", "-c", command, "/dev/null"]
  return ["-q", "/dev/null", "bash", "-lc", command]
}

async function waitForScreenReady(logDir: string, name: string) {
  const logPath = path.join(logDir, "screenlog.0")
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (!(await screenExists(name))) throw new Error(`Claude TUI screen exited before it became ready: ${name}`)
    const log = await readFile(logPath, "utf8").catch(() => "")
    if (log.includes("Claude") && log.includes("Code")) return
    await sleep(250)
  }
  throw new Error(`timed out waiting for Claude TUI screen to become ready: ${name}`)
}

async function screenStuff(name: string, text: string) {
  await execFileAsync("screen", ["-S", name, "-p", "0", "-X", "stuff", text])
}

async function submitScreenPrompt(name: string, prompt: string) {
  await screenStuff(name, prompt)
  await sleep(250)
  await screenStuff(name, "\r")
}

async function screenQuit(name: string) {
  await execFileAsync("screen", ["-S", name, "-p", "0", "-X", "quit"]).catch(() => {})
}

async function screenExists(name: string): Promise<boolean> {
  try {
    const { stdout } = await execFileAsync("screen", ["-ls"])
    return stdout.includes(`.${name}`)
  } catch (error) {
    const output = typeof error === "object" && error && "stdout" in error
      ? String((error as { stdout?: unknown }).stdout ?? "")
      : ""
    return output.includes(`.${name}`)
  }
}

async function readClaudeHookEvents(file: string): Promise<ClaudeHookEvent[]> {
  const raw = await readFile(file, "utf8").catch(() => "")
  return raw
    .split("\n")
    .map((line, index) => ({ line, index }))
    .filter(({ line }) => line.trim())
    .map(({ line, index }) => {
      try {
        const value = JSON.parse(line) as Omit<ClaudeHookEvent, "index">
        return { ...value, index }
      } catch {
        return { index, hook_event_name: "parse_error" }
      }
    })
}

async function drainAssistantText(transcriptPath: string, offsets: Map<string, number>): Promise<string> {
  const { text, lineCount } = await readAssistantTextAfterOffset(transcriptPath, offsets.get(transcriptPath) ?? 0)
  offsets.set(transcriptPath, lineCount)
  return text
}

async function waitForAssistantText(transcriptPath: string, offsets: Map<string, number>): Promise<string> {
  const start = offsets.get(transcriptPath) ?? 0
  const deadline = Date.now() + 5_000
  let latestLineCount = start
  while (Date.now() < deadline) {
    const { text, lineCount } = await readAssistantTextAfterOffset(transcriptPath, start)
    latestLineCount = Math.max(latestLineCount, lineCount)
    if (text.trim()) {
      offsets.set(transcriptPath, lineCount)
      return text
    }
    await sleep(200)
  }
  offsets.set(transcriptPath, latestLineCount)
  return ""
}

async function readAssistantTextAfterOffset(transcriptPath: string, start: number): Promise<{ text: string; lineCount: number }> {
  const raw = await readFile(transcriptPath, "utf8").catch(() => "")
  const lines = raw.split("\n").filter((line) => line.trim())
  const texts: string[] = []
  for (const line of lines.slice(start)) {
    try {
      const entry = JSON.parse(line)
      if (isAssistantTranscriptEntry(entry)) {
        const text = collectTextValues(entry).join("\n").trim()
        if (text) texts.push(text)
      }
    } catch {}
  }
  return { text: texts.join("\n"), lineCount: lines.length }
}

function isAssistantTranscriptEntry(value: unknown): boolean {
  if (!value || typeof value !== "object") return false
  const record = value as Record<string, unknown>
  if (record.type === "assistant" || record.role === "assistant") return true
  const message = record.message
  return Boolean(message && typeof message === "object" && (message as Record<string, unknown>).role === "assistant")
}

function collectTextValues(value: unknown): string[] {
  if (!value || typeof value !== "object") return []
  if (Array.isArray(value)) return value.flatMap((entry) => collectTextValues(entry))
  const record = value as Record<string, unknown>
  const text = typeof record.text === "string" ? [record.text] : []
  return text.concat(Object.values(record).flatMap((entry) => collectTextValues(entry)))
}

function extractClaudeNativePromptAttachments(prompt: string, cwd: string): PromptAttachmentPart[] {
  const candidates = new Set<string>()
  for (const match of prompt.matchAll(/(?:^|\s)@(?:"([^"]+)"|'([^']+)'|([^\s]+))/g)) {
    const raw = match[1] ?? match[2] ?? match[3] ?? ""
    const candidate = trimAttachmentToken(raw)
    if (candidate) candidates.add(candidate)
  }
  return Array.from(candidates).flatMap((candidate) => {
    const classified = classifyPromptAttachment(resolveClaudeAttachmentPath(candidate, cwd))
    if (!classified) return []
    return [{
      url: classified.path,
      mime: classified.mime,
      filename: classified.filename,
    }]
  })
}

function trimAttachmentToken(value: string): string {
  return value.trim().replace(/[),.;:!?]+$/g, "")
}

function resolveClaudeAttachmentPath(value: string, cwd: string): string {
  if (value.startsWith("file://")) return fileURLToPath(value)
  if (value.startsWith("~/")) return path.join(homedir(), value.slice(2))
  return path.resolve(cwd, value)
}

async function formatClaudeAttachmentContext(
  attachments: PromptAttachmentPart[],
  attachmentContextDir: string,
): Promise<string> {
  if (attachments.length === 0) return ""
  await mkdir(attachmentContextDir, { recursive: true })
  const blocks = await Promise.all(attachments.map((attachment, index) =>
    formatClaudeAttachmentBlock(attachment, index, attachmentContextDir),
  ))
  return [
    "The user included prompt attachments. Treat them as part of the current user request.",
    ...blocks,
  ].filter(Boolean).join("\n\n")
}

async function formatClaudeAttachmentBlock(
  attachment: PromptAttachmentPart,
  index: number,
  attachmentContextDir: string,
): Promise<string> {
  const displayName = attachment.filename || `attachment-${index + 1}`
  const attachmentPath = await materializeClaudeAttachmentPath(attachment, index, attachmentContextDir)
  const pieces = [
    `Attachment ${index + 1}: ${displayName}`,
    `MIME: ${attachment.mime}`,
    ...(attachmentPath ? [`Path: ${attachmentPath}`] : []),
  ]
  const text = await readClaudeTextAttachment(attachment, attachmentPath)
  if (text) {
    pieces.push("", "Content:", "```", text, "```")
  } else if (attachmentPath) {
    pieces.push("", "The attachment is available on disk at the path above.")
  } else {
    pieces.push("", "The attachment content is not available to the Claude native bridge.")
  }
  return pieces.join("\n")
}

async function materializeClaudeAttachmentPath(
  attachment: PromptAttachmentPart,
  index: number,
  attachmentContextDir: string,
): Promise<string | null> {
  const localPath = localAttachmentPath(attachment.url)
  if (localPath) return localPath
  if (!attachment.contents_base64) return null
  const filename = safeAttachmentFilename(attachment.filename, attachment.mime, index)
  const materialized = path.join(attachmentContextDir, filename)
  await writeFile(materialized, Buffer.from(attachment.contents_base64, "base64"))
  return materialized
}

async function readClaudeTextAttachment(
  attachment: PromptAttachmentPart,
  attachmentPath: string | null,
): Promise<string | null> {
  const bytes = attachment.contents_base64
    ? Buffer.from(attachment.contents_base64, "base64")
    : attachmentPath && isClaudeTextAttachment(attachment)
      ? await readFile(attachmentPath).catch(() => null)
      : null
  if (!bytes || bytes.length > CLAUDE_ATTACHMENT_CONTEXT_BYTES || !isClaudeTextAttachment(attachment)) {
    return null
  }
  return bytes.toString("utf8")
}

function isClaudeTextAttachment(attachment: PromptAttachmentPart): boolean {
  if (attachment.mime.startsWith("text/")) return true
  if (attachment.mime === "application/json" || attachment.mime.endsWith("+json")) return true
  const filename = attachment.filename?.toLowerCase() ?? ""
  return /\.(md|txt|json|jsonl|csv|ts|tsx|js|jsx|mjs|py|rs|go|java|rb|sh|zsh|yaml|yml|toml|xml|html|css|scss|sql|log)$/.test(filename)
}

function safeAttachmentFilename(filename: string | null | undefined, mime: string, index: number): string {
  const fallback = `attachment-${index + 1}${extensionForMime(mime)}`
  const base = path.basename(filename || fallback).replace(/[^A-Za-z0-9._-]/g, "_")
  return `${index + 1}-${base || fallback}`
}

function extensionForMime(mime: string): string {
  if (mime === "image/png") return ".png"
  if (mime === "image/jpeg") return ".jpg"
  if (mime === "image/gif") return ".gif"
  if (mime === "image/webp") return ".webp"
  if (mime === "application/pdf") return ".pdf"
  if (mime === "application/json") return ".json"
  if (mime.startsWith("text/")) return ".txt"
  return ".bin"
}

function joinClaudeAdditionalContext(...parts: string[]): string {
  return parts.map((part) => part.trim()).filter(Boolean).join("\n\n")
}

function extractHiddenInstructions(prompt: string): string {
  const start = prompt.indexOf(hiddenInstructionsStart)
  if (start < 0) return ""
  const end = prompt.indexOf(hiddenInstructionsEnd, start)
  if (end < 0) return prompt.slice(start)
  return prompt.slice(start + hiddenInstructionsStart.length, end).trim()
}

function promptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  return session.prompt_states?.[agentId]?.active_prompt
    ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
}

function extractSubmittedPromptId(response: Record<string, unknown>, agentId: string): string | null {
  const payload = response.PromptSubmitted as { outcome?: Record<string, unknown>; session?: RuntimeSession } | undefined
  if (!payload) return null
  for (const variant of Object.values(payload.outcome ?? {})) {
    const prompt = variant && typeof variant === "object"
      ? (variant as { prompt?: PromptQueueItem | null }).prompt
      : null
    if (prompt?.id) return prompt.id
  }
  const session = payload.session ? normalizeRuntimeSession(payload.session) : null
  return session ? promptForAgent(session, agentId)?.id ?? null : null
}

async function sessionState(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  return normalizeRuntimeSession(expectVariant<{ session: RuntimeSession }>(response, "SessionState").session)
}

async function createSession(
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
  alias: string | undefined,
  model: string,
  effort: string,
  mode: "build" | "plan",
  permissions: "required" | "yolo",
): Promise<{ session: RuntimeSession; agent: AgentInstance }> {
  const response = await client.send<Record<string, unknown>>(createSessionRequest(workspace, worktree, alias, {
    provider: "claude",
    model,
    effort,
    execution_mode: mode,
    permission_level: permissions,
  }))
  const payload = expectVariant<{ session: RuntimeSession; agent: AgentInstance }>(response, "SessionCreated")
  return { session: normalizeRuntimeSession(payload.session), agent: payload.agent }
}

async function resolveSession(
  client: LocalIpcClient,
  sessionRef: string,
  workspace: string,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(resolveSessionRequest(sessionRef, workspace))
  return normalizeRuntimeSession(expectVariant<{ session: RuntimeSession }>(response, "SessionResolved").session)
}

async function attachToSession(client: LocalIpcClient, sessionId: string, clientId: string): Promise<RuntimeAttachment> {
  const response = await client.send<Record<string, unknown>>(attachToSessionRequest(sessionId, clientId))
  return expectVariant<{ attachment: RuntimeAttachment }>(response, "SessionAttached").attachment
}

async function spawnClaudeAgent(
  client: LocalIpcClient,
  sessionId: string,
  alias: string | undefined,
  model: string,
  effort: string,
  worktree: string,
  mode: "build" | "plan",
  permissions: "required" | "yolo",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    spawnAgentRequest(sessionId, "claude", alias, model, worktree, effort, mode, permissions),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
}

async function maybeAliasAgent(
  client: LocalIpcClient,
  sessionId: string,
  agent: AgentInstance,
  alias: string | undefined,
): Promise<AgentInstance> {
  if (!alias || agent.alias === alias) return agent
  const response = await client.send<Record<string, unknown>>(aliasAgentRequest(sessionId, agent.id, alias))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentAliased").agent
}

async function launchClaudeNativeRun(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  model: string,
  effort: string,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(
    launchProviderRunRequest(sessionId, "claude", "default", model, effort, agentId, {
      structuredEndpoint: `native://claude/${process.pid}`,
      nativeTui: true,
    }),
  )
  return "ProviderRunLaunched" in response
    ? expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched").provider_run
    : expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunchAccepted").provider_run
}

async function launchClaudeRemoteRenderedRun(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  model: string,
  effort: string,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(
    launchProviderRunRequest(sessionId, "claude", "default", model, effort, agentId, {
      nativeTui: true,
    }),
  )
  return "ProviderRunLaunched" in response
    ? expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched").provider_run
    : expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunchAccepted").provider_run
}

async function waitForProviderRunReady(client: LocalIpcClient, providerRunId: string): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + 30_000
  let latest: RuntimeProviderRun | null = null
  while (Date.now() < deadline) {
    latest = expectVariant<{ provider_run: RuntimeProviderRun }>(
      await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId)),
      "ProviderRun",
    ).provider_run
    if (latest.state === "Running") return latest
    if (latest.state === "Ended") throw new Error(`Claude provider run ended before native TUI was ready: ${providerRunId}`)
    await sleep(250)
  }
  throw new Error(`timed out waiting for Claude provider run ${providerRunId}; latest state ${latest?.state ?? "unknown"}`)
}

async function waitForRemoteRenderedRunExit(client: LocalIpcClient, providerRunId: string): Promise<void> {
  while (true) {
    const run = expectVariant<{ provider_run: RuntimeProviderRun }>(
      await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId)),
      "ProviderRun",
    ).provider_run
    if (run.state === "Ended") return
    await sleep(1_000)
  }
}

function startKernelPumpLoop(client: LocalIpcClient, sessionId: string, attachmentId: string): { stop: () => void } {
  let stopped = false
  const loop = async () => {
    while (!stopped) {
      await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => ({}))
      await sleep(500)
    }
  }
  void loop()
  return {
    stop: () => {
      stopped = true
    },
  }
}

function startRemoteRenderedPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
): { stop: () => void } {
  let stopped = false
  const loop = async () => {
    while (!stopped) {
      const response = await client
        .send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
        .catch(() => ({}))
      const records = "TerminalOutput" in response ? (response.TerminalOutput as { records?: unknown[] }).records : null
      writeRemoteRenderedTerminalRecords(records, providerRunId)
      await sleep(250)
    }
  }
  void loop()
  return {
    stop: () => {
      stopped = true
    },
  }
}

function writeRemoteRenderedTerminalRecords(records: unknown, providerRunId: string) {
  if (!Array.isArray(records)) return
  for (const record of records) {
    if (!record || typeof record !== "object") continue
    const payload = record as { provider_run_id?: unknown; bytes?: unknown }
    if (payload.provider_run_id !== providerRunId) continue
    const bytes = Array.isArray(payload.bytes) ? Buffer.from(payload.bytes as number[]) : null
    if (bytes?.length) process.stdout.write(bytes)
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}

function parseMode(value: string): "build" | "plan" {
  if (value === "build" || value === "plan") return value
  throw new Error(`invalid mode ${value}; expected build or plan`)
}

function parsePermissions(value: string): "required" | "yolo" {
  if (value === "required" || value === "yolo") return value
  throw new Error(`invalid permissions ${value}; expected required or yolo`)
}

function parseKernelPort(value: string, flag: string): string {
  const port = Number(value)
  if (!Number.isInteger(port) || port <= 0 || port > 65_535) {
    throw new Error(`invalid ${flag} value ${value}`)
  }
  return String(port)
}

function defaultKernelEndpoint(port?: string): string {
  return `ws://127.0.0.1:${port ?? "43119"}/kernel`
}

function debugNativeClaude(label: string, payload: unknown) {
  if (!process.env.ARROBA_CLAUDE_NATIVE_DEBUG) return
  const line = `[arroba claude native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.ARROBA_CLAUDE_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}

async function inferWorkspaceTargetsFromLaunchDirectory(cwd: string): Promise<{ workspace: string; worktree: string }> {
  try {
    const gitDirResult = await execFileAsync("git", ["rev-parse", "--git-dir"], { cwd })
    const commonDirResult = await execFileAsync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd })
    const worktree = gitDirResult.stdout.trim()
      ? (await execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd })).stdout.trim()
      : cwd
    const commonDir = commonDirResult.stdout.trim()
    if (!worktree) return { workspace: cwd, worktree: cwd }
    const workspace = commonDir.endsWith("/.git") ? commonDir.slice(0, -"/.git".length) : worktree
    return { workspace, worktree }
  } catch {
    return { workspace: cwd, worktree: cwd }
  }
}

function shellQuote(value: string): string {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}
