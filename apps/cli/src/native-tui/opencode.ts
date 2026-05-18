import { spawn, execFile } from "node:child_process"
import { appendFileSync } from "node:fs"
import http, { type IncomingMessage, type ServerResponse } from "node:http"
import net from "node:net"
import path from "node:path"
import process from "node:process"
import { Readable, Transform } from "node:stream"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"

import {
  type PromptAttachmentPart,
  type RuntimeProviderRun,
  type RuntimeSession,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  cancelActivePromptRequest,
  getSessionStateRequest,
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
  respondToInteractionRequest,
  submitPromptRequest,
  updateProviderRunSelectionRequest,
} from "../ipc-requests.js"
import {
  preparePromptAttachmentsForSubmit,
  promptAttachmentTransferIsForced,
} from "../prompt-attachment-transfer.js"
import { grantNativeCapabilities } from "./capability-grants.js"
import { hiddenInstructionsStart, redactHiddenInstructions } from "./hidden-instructions.js"
import {
  getNativeProviderRun,
  requestNativeProviderRunLaunch,
} from "./provider-run-control.js"
import { bridgeRemoteNativeProviderEndpoint } from "./remote-endpoint-bridge.js"
import {
  attachNativeSession,
  createNativeSession,
  prepareCreatedNativeAgent,
  resolveNativeSession,
  spawnNativeAgent,
} from "./session-control.js"

const execFileAsync = promisify(execFile)

type NativeOpenCodeOptions = {
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
  machineRef?: string
  sliceRef?: string
  alias?: string
  agentAlias?: string
  mode: "build" | "plan"
  permissions: "required" | "yolo"
  serverInKernel: boolean
  grantMcps: string[]
  grantSkills: string[]
}

type OpenCodePromptBody = {
  parts?: unknown
  text?: unknown
  prompt?: unknown
  model?: unknown
  variant?: unknown
  agent?: unknown
}

type NativeProviderSelection = {
  model?: string
  variant?: string
  agent?: string
}

type OpenCodeProxyState = {
  providerRunId: string | null
  providerSessionId: string | null
  providerRunLocal: boolean
  lastNativeSelection: NativeProviderSelection | null
}

export async function runOpenCodeNativeTui(args: string[]): Promise<void> {
  const options = parseNativeOpenCodeArgs(args)
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

  let proxy: http.Server | null = null
  let openCodeServer: ReturnType<typeof spawn> | null = null
  let endpointBridge: { close: () => Promise<void> } | null = null
  let pump: { stop: () => void } | null = null
  try {
    const remotePlacement = Boolean(options.machineRef || options.sliceRef)
    const created = options.sessionRef
      ? null
      : await createNativeSession(client, workspace, worktree, options.alias, {
        provider: "opencode",
        model: "default",
        effort: null,
        execution_mode: options.mode,
        permission_level: options.permissions,
      }, options.sliceRef)
    const session = created?.session ?? (options.sessionRef
      ? await resolveNativeSession(client, options.sessionRef, workspace)
      : (() => {
        throw new Error("missing OpenCode session")
      })())
    const attachment = await attachNativeSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await prepareCreatedNativeAgent(client, session.id, created.agent, options.agentAlias, options.machineRef)
      : await spawnNativeAgent(client, session.id, "opencode", options.agentAlias, "default", undefined, null, options.mode, options.permissions, options.machineRef, options.sliceRef)
    await grantNativeCapabilities(client, workspace, agent.id, options.grantMcps, options.grantSkills)
    const proxyState: OpenCodeProxyState = {
      providerRunId: null,
      providerSessionId: null,
      providerRunLocal: true,
      lastNativeSelection: null,
    }
    let upstreamBaseUrl: string
    let run: RuntimeProviderRun | null = null
    if (options.serverInKernel) {
      const launched = await requestNativeProviderRunLaunch(client, {
        sessionId: session.id,
        provider: "opencode",
        model: "default",
        effort: "",
        agentId: agent.id,
      })
      run = !remotePlacement && launched.session_id === session.id
        ? await waitForOpenCodeRunReady(client, launched.id)
        : launched
      if (!run.structured_endpoint) {
        throw new Error("OpenCode managed native server did not expose an endpoint")
      }
      upstreamBaseUrl = run.structured_endpoint
    } else {
      upstreamBaseUrl = `http://127.0.0.1:${await reservePort()}`
      openCodeServer = await startOpenCodeServer(upstreamBaseUrl, worktree)
    }
    const bridgedEndpoint = await bridgeRemoteNativeProviderEndpoint(upstreamBaseUrl, "OpenCode")
    upstreamBaseUrl = bridgedEndpoint.endpoint
    endpointBridge = bridgedEndpoint
    proxy = await startOpenCodeProxy({
      upstreamBaseUrl,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      inlineLocalAttachments: Boolean(options.relayUrl) || remotePlacement || promptAttachmentTransferIsForced(),
    }, proxyState)
    const proxyAddress = proxy.address()
    if (!proxyAddress || typeof proxyAddress === "string") {
      throw new Error("OpenCode proxy did not expose a TCP port")
    }
    const proxyUrl = `http://127.0.0.1:${proxyAddress.port}`
    if (!options.serverInKernel) {
      const launched = await requestNativeProviderRunLaunch(client, {
        sessionId: session.id,
        provider: "opencode",
        model: "default",
        effort: "",
        agentId: agent.id,
        native: { structuredEndpoint: proxyUrl },
      })
      run = !remotePlacement && launched.session_id === session.id
        ? await waitForOpenCodeRunReady(client, launched.id)
        : launched
    }
    if (!run) {
      throw new Error("OpenCode provider run was not launched")
    }
    const providerSessionId = run.provider_session_id
    if (!providerSessionId) {
      throw new Error("OpenCode provider run did not expose provider_session_id")
    }
    proxyState.providerRunId = run.id
    proxyState.providerSessionId = providerSessionId
    proxyState.providerRunLocal = run.session_id === session.id
    process.stderr.write([
      "[arroba opencode native-tui]",
      `  arroba session: ${session.id}${session.alias ? ` (${session.alias})` : ""}`,
      `  arroba agent:   ${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`,
      `  provider run:   ${run.id}`,
      `  opencode sess:  ${providerSessionId}`,
      `  opencode server:${upstreamBaseUrl}`,
      `  proxy:          ${proxyUrl}`,
      "  prompt policy:  native prompts pass through; Arroba observes the session",
      "",
    ].join("\n"))
    pump = startKernelPumpLoop(client, session.id, attachment.id)

    await runOpenCodeAttach({
      proxyUrl,
      providerSessionId,
      workingDirectory: worktree,
    })
  } finally {
    pump?.stop()
    if (proxy) {
      await new Promise<void>((resolve) => proxy!.close(() => resolve()))
    }
    if (openCodeServer && openCodeServer.exitCode == null) {
      openCodeServer.kill("SIGTERM")
      await Promise.race([
        new Promise((resolve) => openCodeServer?.once("exit", resolve)),
        sleep(2_000),
      ])
      if (openCodeServer.exitCode == null) openCodeServer.kill("SIGKILL")
    }
    await endpointBridge?.close()
    await client.close()
  }
}

function parseNativeOpenCodeArgs(args: string[]): NativeOpenCodeOptions {
  const options: NativeOpenCodeOptions = {
    clientId: `arroba-opencode-native-${process.pid}`,
    mode: "build",
    permissions: "yolo",
    serverInKernel: false,
    grantMcps: [],
    grantSkills: [],
  }
  const positional: string[] = []

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
      case "--machine":
      case "--kernel-ref":
        options.machineRef = next()
        break
      case "--slice":
        options.sliceRef = next()
        break
      case "--alias":
        options.alias = next()
        break
      case "--agent-alias":
        options.agentAlias = next()
        break
      case "--mode":
        options.mode = parseMode(next())
        break
      case "--permissions":
        options.permissions = parsePermissions(next())
        break
      case "--server-in-kernel":
        options.serverInKernel = true
        break
      case "--grant-mcp":
        options.grantMcps.push(next())
        break
      case "--grant-skill":
        options.grantSkills.push(next())
        break
      case "--help":
      case "-h":
        printNativeOpenCodeUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) {
          throw new Error(`unknown opencode argument ${arg}`)
        }
        positional.push(arg)
    }
  }

  if (positional.length > 1) {
    throw new Error("usage: arroba opencode [session-ref]")
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
  if (options.relayUrl && options.kernelPort) {
    throw new Error("--relay-url cannot be used together with --kernel-port")
  }
  if (options.kernelUrl && options.kernelPort) {
    throw new Error("--kernel-url cannot be used together with --kernel-port")
  }
  if (options.socketPath && options.kernelPort) {
    throw new Error("--socket cannot be used together with --kernel-port")
  }
  if (options.machineRef && options.sliceRef) {
    throw new Error("--machine and --slice cannot be used together")
  }
  if (options.machineRef && !options.serverInKernel) {
    throw new Error("--machine requires --server-in-kernel so the OpenCode server is launched by the worker kernel")
  }
  if (options.sliceRef && !options.serverInKernel) {
    throw new Error("--slice requires --server-in-kernel so the OpenCode server is launched by the slice worker kernel")
  }
  if (positional[0] !== undefined) {
    options.sessionRef = positional[0]
  }
  return options
}

function printNativeOpenCodeUsage() {
  process.stdout.write([
    "usage: arroba opencode [session-ref] [--socket PATH|--kernel-url URL|--kernel-port PORT] [--mode build|plan] [--permissions required|yolo]",
    "       arroba opencode [session-ref] --relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)",
    "",
    "placement:",
    "  --machine, --kernel-ref REF       Run the Arroba agent/provider on a remote worker kernel",
    "  --slice REF                       Run the Arroba agent/provider on a home-managed slice worker",
    "",
    "behavior:",
    "  --grant-mcp NAME                Grant an installed Arroba MCP to the native agent before provider launch",
    "  --grant-skill NAME              Grant an installed Arroba skill to the native agent before provider launch",
    "  creates a new Arroba agent in the selected session and launches native `opencode attach` for it.",
  ].join("\n") + "\n")
}

function parseMode(value: string): "build" | "plan" {
  if (value === "build" || value === "plan") return value
  throw new Error("--mode must be build or plan")
}

function parsePermissions(value: string): "required" | "yolo" {
  if (value === "required" || value === "yolo") return value
  throw new Error("--permissions must be required or yolo")
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

function defaultKernelEndpoint(kernelPort?: string): string {
  if (process.env.ARROBA_KERNEL_URL) {
    return process.env.ARROBA_KERNEL_URL
  }
  const host = process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"
  const port = kernelPort ?? process.env.ARROBA_KERNEL_PORT ?? "43119"
  return `ws://${host}:${port}/kernel`
}

function parseKernelPort(value: string, arg: string): string {
  if (!/^\d+$/.test(value)) {
    throw new Error(`${arg} must be a TCP port`)
  }
  const port = Number(value)
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`${arg} must be between 1 and 65535`)
  }
  return String(port)
}

async function updateProviderRunSelection(
  client: LocalIpcClient,
  sessionId: string,
  providerRunId: string,
  selection: NativeProviderSelection,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(
    updateProviderRunSelectionRequest(sessionId, providerRunId, {
      model: selection.model ?? null,
      variant: selection.variant ?? null,
      clearVariant: selection.variant === "",
    }),
  )
  return expectVariant<{ provider_run: RuntimeProviderRun }>(
    response,
    "ProviderRunSelectionUpdated",
  ).provider_run
}

async function waitForOpenCodeRunReady(
  client: LocalIpcClient,
  providerRunId: string,
): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + 60_000
  let latest: RuntimeProviderRun | null = null
  while (Date.now() < deadline) {
    latest = await getNativeProviderRun(client, providerRunId)
    if (
      latest.state === "Running"
      && latest.structured_endpoint
      && latest.provider_session_id
    ) {
      return latest
    }
    if (latest.state === "Ended") {
      throw new Error(`OpenCode provider run ended before attach was ready: ${providerRunId}`)
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for OpenCode provider run to become ready: ${providerRunId} (${latest?.state ?? "unknown"})`)
}

function assertLocalStructuredEndpoint(endpoint: string) {
  const url = new URL(endpoint)
  if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
    throw new Error(`native OpenCode TUI mode only supports local provider endpoints for now; got ${endpoint}`)
  }
}

async function reservePort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("port reservation did not expose a TCP address")))
        return
      }
      const port = address.port
      server.close(() => resolve(port))
    })
  })
}

async function startOpenCodeServer(baseUrl: string, workingDirectory: string) {
  assertLocalStructuredEndpoint(baseUrl)
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
  const url = new URL(baseUrl)
  const child = spawn(executable, [
    "serve",
    "--hostname",
    url.hostname,
    "--port",
    url.port,
  ], {
    cwd: workingDirectory,
    stdio: ["ignore", "ignore", "inherit"],
    env: process.env,
  })
  child.once("error", (error) => {
    throw error
  })
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (await openCodeHealthIsReady(baseUrl)) return child
    if (child.exitCode !== null) {
      throw new Error(`opencode serve exited before becoming ready with ${child.exitCode}`)
    }
    await sleep(100)
  }
  throw new Error(`timed out waiting for opencode serve at ${baseUrl}`)
}

async function openCodeHealthIsReady(baseUrl: string): Promise<boolean> {
  try {
    const response = await fetch(new URL("/global/health", baseUrl))
    return response.ok
  } catch {
    return false
  }
}

async function startOpenCodeProxy(options: {
  upstreamBaseUrl: string
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  inlineLocalAttachments: boolean
}, state: OpenCodeProxyState): Promise<http.Server> {
  const server = http.createServer((request, response) => {
    handleProxyRequest(request, response, options, state).catch((error) => {
      if (!response.headersSent) {
        response.writeHead(502, { "content-type": "application/json" })
      }
      response.end(JSON.stringify({ error: formatError(error) }))
    })
  })
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject)
      debugNativeMutation("proxy_listening", { agentId: options.agentId })
      resolve()
    })
  })
  return server
}

async function handleProxyRequest(
  request: IncomingMessage,
  response: ServerResponse,
  options: {
    upstreamBaseUrl: string
    client: LocalIpcClient
    sessionId: string
    attachmentId: string
    agentId: string
    inlineLocalAttachments: boolean
  },
  state: OpenCodeProxyState,
): Promise<void> {
  const method = request.method ?? "GET"
  const path = request.url ?? "/"
  const isKernelRequest = request.headers["x-arroba-provider-client"] === "kernel"
  debugNativeMutation("request", { method, path })
  const promptMatch = method === "POST" ? path.match(/^\/session\/([^/]+)\/(?:message|prompt_async)(?:\?.*)?$/) : null
  if (promptMatch && isKernelRequest) {
    await proxyToOpenCode(request, response, options.upstreamBaseUrl, false)
    return
  }
  if (promptMatch) {
    if (!state.providerRunId) {
      response.writeHead(503, { "content-type": "application/json" })
      response.end(JSON.stringify({ error: "OpenCode provider run is not bound yet" }))
      return
    }
    const body = await readRequestJson<OpenCodePromptBody>(request)
    debugNativeMutation(path.includes("/prompt_async") ? "prompt_async" : "message", body)
    const prompt = extractPromptText(body)
    const attachments = await preparePromptAttachmentsForSubmit(extractPromptAttachments(body), {
      inlineLocalFiles: options.inlineLocalAttachments,
    })
    if (attachments.length > 0) {
      debugNativeMutation("native_prompt_attachments_observed", {
        path,
        attachmentCount: attachments.length,
      })
    }
    const selection = extractOpenCodeSelection(body)
    if (selection) {
      debugNativeMutation("selection_observed", selection)
    }
    const run = state.providerRunLocal
      ? await getNativeProviderRun(options.client, state.providerRunId)
      : null
    if (selection && run && shouldApplyNativeSelection(selection, state.lastNativeSelection, run)) {
      const updated = await updateProviderRunSelection(
        options.client,
        options.sessionId,
        state.providerRunId,
        selection,
      )
      debugNativeMutation("selection_applied", {
        model: updated.model,
        variant: updated.variant,
      })
    } else if (selection && run) {
      debugNativeMutation("selection_ignored_as_stale", {
        incoming: selection,
        current: {
          model: run.model,
          variant: run.variant,
        },
      })
    } else if (selection) {
      debugNativeMutation("selection_observed_on_remote_run", {
        model: selection.model,
        variant: selection.variant,
      })
    }
    if (selection?.model || selection?.variant) {
      state.lastNativeSelection = compactSelection(selection)
    }
    await options.client.send<Record<string, unknown>>(
      submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
    )
    response.writeHead(204, { "content-length": "0" })
    response.end()
    return
  }

  const abortMatch = method === "POST" ? path.match(/^\/session\/([^/]+)\/abort(?:\?.*)?$/) : null
  if (abortMatch && !isKernelRequest) {
    await options.client.send<Record<string, unknown>>(
      cancelActivePromptRequest(options.sessionId, options.attachmentId),
    )
    response.writeHead(204, { "content-length": "0" })
    response.end()
    return
  }

  if (!isKernelRequest && method === "POST" && /^\/session\/[^/]+\/permissions\//.test(path)) {
    const body = await readRequestJson<Record<string, unknown>>(request).catch(() => ({}))
    const choiceId = openCodeNativePermissionChoice(body)
    const resolved = await resolveActiveNativePermissionInteraction(options.client, options.sessionId, options.agentId, choiceId)
    if (!resolved) {
      response.writeHead(409, { "content-type": "application/json" })
      response.end(JSON.stringify({
        error: "No active Arroba permission interaction is available for this OpenCode native TUI response.",
      }))
      return
    }
    response.writeHead(204, { "content-length": "0" })
    response.end()
    return
  }

  if (!isKernelRequest && method === "GET" && new URL(path, options.upstreamBaseUrl).pathname === "/event") {
    await proxyOpenCodeEventsForNativeTui(request, response, options.upstreamBaseUrl, state)
    return
  }

  await proxyToOpenCode(request, response, options.upstreamBaseUrl, !isKernelRequest)
}

async function resolveActiveNativePermissionInteraction(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  choiceId: string,
): Promise<boolean> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
  const interaction = session.active_interactions?.find((entry) =>
    entry.agent_id === agentId && entry.kind === "permission")
  if (!interaction) return false
  await client.send<Record<string, unknown>>(
    respondToInteractionRequest(sessionId, interaction.id, choiceId),
  )
  return true
}

function openCodeNativePermissionChoice(body: Record<string, unknown>): string {
  const raw = [
    body.response,
    body.decision,
    body.choice,
    body.action,
    body.status,
  ].find((value) => typeof value === "string")
  const value = typeof raw === "string" ? raw.toLowerCase() : ""
  if (value.includes("always") || value.includes("session")) return "allow_session"
  if (value.includes("reject") || value.includes("deny") || value.includes("decline")) return "deny"
  return "allow_once"
}

async function proxyOpenCodeEventsForNativeTui(
  request: IncomingMessage,
  response: ServerResponse,
  upstreamBaseUrl: string,
  state: OpenCodeProxyState,
): Promise<void> {
  const target = new URL(request.url ?? "/", upstreamBaseUrl)
  const headers = requestHeadersForFetch(request)
  const upstream = await fetch(target, {
    method: request.method ?? "GET",
    headers,
  })

  response.statusCode = upstream.status
  upstream.headers.forEach((value, key) => {
    const lowerKey = key.toLowerCase()
    if (lowerKey !== "content-encoding" && lowerKey !== "content-length") {
      response.setHeader(key, value)
    }
  })
  if (!upstream.body) {
    response.end()
    return
  }

  let carry = ""
  let refreshCounter = 0
  let refreshInFlight = false
  const refreshFromProviderSession = async () => {
    const providerSessionId = state.providerSessionId
    if (!providerSessionId || refreshInFlight || response.destroyed) return
    refreshInFlight = true
    try {
      refreshCounter = await emitNativeTranscriptRefresh({
        response,
        upstreamBaseUrl,
        sessionId: providerSessionId,
        directory: target.searchParams.get("directory"),
        counter: refreshCounter,
      })
    } finally {
      refreshInFlight = false
    }
  }
  const refreshTimer = setInterval(() => {
    void refreshFromProviderSession().catch((error) => {
      debugNativeMutation("native_refresh_timer_failed", { error: formatError(error) })
    })
  }, 1_000)
  response.once("close", () => clearInterval(refreshTimer))
  try {
    for await (const chunk of Readable.fromWeb(upstream.body as never)) {
      carry += Buffer.isBuffer(chunk) ? chunk.toString("utf8") : String(chunk)
      while (true) {
        const separator = findSseFrameSeparator(carry)
        if (!separator) break
        const frame = carry.slice(0, separator.index)
        const delimiter = carry.slice(separator.index, separator.index + separator.length)
        const redactedFrame = redactHiddenInstructions(frame)
        response.write(redactedFrame)
        response.write(delimiter)
        const sessionId = sessionIdNeedingNativeRefresh(frame)
        if (sessionId) {
          refreshCounter = await emitNativeTranscriptRefresh({
            response,
            upstreamBaseUrl,
            sessionId,
            directory: target.searchParams.get("directory"),
            counter: refreshCounter,
          })
        }
        carry = carry.slice(separator.index + separator.length)
      }
    }
    if (carry) {
      response.write(redactHiddenInstructions(carry))
    }
  } finally {
    clearInterval(refreshTimer)
  }
  response.end()
}

async function proxyToOpenCode(
  request: IncomingMessage,
  response: ServerResponse,
  upstreamBaseUrl: string,
  redactForNativeTui = false,
): Promise<void> {
  const method = request.method ?? "GET"
  const target = new URL(request.url ?? "/", upstreamBaseUrl)
  const headers = requestHeadersForFetch(request)
  const init: RequestInit = {
    method,
    headers,
  }
  if (method !== "GET" && method !== "HEAD") {
    const body = await readRequestBuffer(request)
    if (body.includes(hiddenInstructionsStart)) {
      debugNativeMutation("hidden_instructions_forwarded", { method, path: request.url ?? "/" })
    }
    if (body.includes("\"type\":\"file\"") || body.includes("\"type\": \"file\"")) {
      debugNativeMutation("attachments_forwarded", { method, path: request.url ?? "/" })
    }
    init.body = body as unknown as BodyInit
  }
  const upstream = await fetch(target, init)

  response.statusCode = upstream.status
  upstream.headers.forEach((value, key) => {
    const lowerKey = key.toLowerCase()
    if (lowerKey !== "content-encoding" && lowerKey !== "content-length") {
      response.setHeader(key, value)
    }
  })
  if (!upstream.body) {
    response.end()
    return
  }
  await new Promise<void>((resolve, reject) => {
    const stream = Readable.fromWeb(upstream.body as never)
    const readable = redactForNativeTui
      ? stream.pipe(target.pathname === "/event"
        ? createSseHiddenInstructionRedactor()
        : createHiddenInstructionRedactor())
      : stream
    readable
      .once("error", reject)
      .once("end", resolve)
      .pipe(response)
  })
}

function requestHeadersForFetch(request: IncomingMessage): Headers {
  const headers = new Headers()
  for (const [key, value] of Object.entries(request.headers)) {
    if (!value || key.toLowerCase() === "host" || key.toLowerCase() === "content-length") continue
    if (Array.isArray(value)) {
      for (const entry of value) headers.append(key, entry)
    } else {
      headers.set(key, value)
    }
  }
  return headers
}

function createSseHiddenInstructionRedactor(): Transform {
  let carry = ""
  return new Transform({
    transform(chunk, _encoding, callback) {
      carry += chunk.toString("utf8")
      while (true) {
        const separator = findSseFrameSeparator(carry)
        if (!separator) break
        const frame = carry.slice(0, separator.index)
        const delimiter = carry.slice(separator.index, separator.index + separator.length)
        this.push(redactHiddenInstructions(frame))
        this.push(delimiter)
        carry = carry.slice(separator.index + separator.length)
      }
      callback()
    },
    flush(callback) {
      this.push(redactHiddenInstructions(carry))
      callback()
    },
  })
}

function findSseFrameSeparator(value: string): { index: number; length: number } | null {
  const candidates = [
    { index: value.indexOf("\r\n\r\n"), length: 4 },
    { index: value.indexOf("\n\n"), length: 2 },
  ].filter((candidate) => candidate.index >= 0)
  if (candidates.length === 0) return null
  candidates.sort((left, right) => left.index - right.index)
  return candidates[0] ?? null
}

function sessionIdNeedingNativeRefresh(frame: string): string | null {
  const payload = sseDataPayload(frame)
  if (!payload) return null
  let event: unknown
  try {
    event = JSON.parse(payload)
  } catch {
    return null
  }
  if (!event || typeof event !== "object") return null
  const record = event as Record<string, unknown>
  const type = typeof record.type === "string" ? record.type : ""
  const properties = record.properties && typeof record.properties === "object"
    ? record.properties as Record<string, unknown>
    : {}
  const sessionId = typeof properties.sessionID === "string" ? properties.sessionID : null
  if (!sessionId) return null
  if (type === "session.idle") return sessionId
  if (type === "session.status") {
    const status = properties.status && typeof properties.status === "object"
      ? properties.status as Record<string, unknown>
      : {}
    return status.type === "idle" ? sessionId : null
  }
  return null
}

function sseDataPayload(frame: string): string | null {
  const lines = frame.split(/\r?\n/)
  const data = lines.flatMap((line) => {
    if (!line.startsWith("data:")) return []
    return [line.slice("data:".length).trimStart()]
  })
  return data.length > 0 ? data.join("\n") : null
}

async function emitNativeTranscriptRefresh(options: {
  response: ServerResponse
  upstreamBaseUrl: string
  sessionId: string
  directory: string | null
  counter: number
}): Promise<number> {
  const url = new URL(`/session/${encodeURIComponent(options.sessionId)}/message`, options.upstreamBaseUrl)
  url.searchParams.set("limit", "100")
  if (options.directory) {
    url.searchParams.set("directory", options.directory)
  }
  const refresh = await fetch(url)
  if (!refresh.ok) {
    debugNativeMutation("native_refresh_failed", {
      sessionId: options.sessionId,
      status: refresh.status,
    })
    return options.counter
  }
  const text = redactHiddenInstructions(await refresh.text())
  let messages: unknown
  try {
    messages = JSON.parse(text)
  } catch (error) {
    debugNativeMutation("native_refresh_parse_failed", {
      sessionId: options.sessionId,
      error: formatError(error),
    })
    return options.counter
  }
  if (!Array.isArray(messages)) return options.counter
  let counter = options.counter
  for (const message of messages) {
    if (!message || typeof message !== "object") continue
    const record = message as Record<string, unknown>
    if (record.info && typeof record.info === "object") {
      counter += 1
      writeSseData(options.response, {
        id: `arroba_native_refresh_${Date.now()}_${counter}`,
        type: "message.updated",
        properties: { info: record.info },
      })
    }
    if (!Array.isArray(record.parts)) continue
    for (const part of record.parts) {
      if (!part || typeof part !== "object") continue
      counter += 1
      writeSseData(options.response, {
        id: `arroba_native_refresh_${Date.now()}_${counter}`,
        type: "message.part.updated",
        properties: { part },
      })
    }
  }
  return counter
}

function writeSseData(response: ServerResponse, payload: unknown) {
  response.write(`data: ${JSON.stringify(payload)}\n\n`)
}

function createHiddenInstructionRedactor(): Transform {
  let carry = ""
  const keepTail = 64
  return new Transform({
    transform(chunk, _encoding, callback) {
      const combined = `${carry}${chunk.toString("utf8")}`
      const redacted = redactHiddenInstructions(combined)
      const startIndex = redacted.lastIndexOf("<<<ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>")
      if (startIndex >= 0) {
        this.push(redacted.slice(0, startIndex))
        carry = redacted.slice(startIndex)
      } else {
        const emitLength = Math.max(0, redacted.length - keepTail)
        this.push(redacted.slice(0, emitLength))
        carry = redacted.slice(emitLength)
      }
      callback()
    },
    flush(callback) {
      this.push(redactHiddenInstructions(carry))
      callback()
    },
  })
}

function extractPromptText(body: OpenCodePromptBody): string {
  const parts = Array.isArray(body.parts) ? body.parts : []
  const textParts = parts.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    return record.type === "text" && typeof record.text === "string" ? [record.text] : []
  })
  if (textParts.length > 0) {
    return textParts.join("\n")
  }
  if (typeof body.text === "string") return body.text
  if (typeof body.prompt === "string") return body.prompt
  return ""
}

function extractPromptAttachments(body: OpenCodePromptBody): PromptAttachmentPart[] {
  const parts = Array.isArray(body.parts) ? body.parts : []
  return parts.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    if (record.type !== "file" || typeof record.url !== "string") return []
    return [{
      url: record.url,
      mime: typeof record.mime === "string" ? record.mime : "application/octet-stream",
      filename: typeof record.filename === "string" ? record.filename : null,
    }]
  })
}

function extractOpenCodeSelection(body: OpenCodePromptBody): { model?: string; variant?: string; agent?: string } | null {
  const selection: { model?: string; variant?: string; agent?: string } = {}
  if (body.model && typeof body.model === "object") {
    const model = body.model as Record<string, unknown>
    const providerId = typeof model.providerID === "string" ? model.providerID : null
    const modelId = typeof model.modelID === "string" ? model.modelID : null
    if (providerId && modelId) {
      selection.model = `${providerId}/${modelId}`
    }
  } else if (typeof body.model === "string") {
    selection.model = body.model
  }
  if (typeof body.variant === "string" && body.variant.trim()) {
    selection.variant = body.variant
  }
  if (typeof body.agent === "string" && body.agent.trim()) {
    selection.agent = body.agent
  }
  return selection.model || selection.variant || selection.agent ? selection : null
}

function shouldApplyNativeSelection(
  incoming: NativeProviderSelection,
  lastNativeSelection: NativeProviderSelection | null,
  run: RuntimeProviderRun,
) {
  const incomingModel = incoming.model?.trim() || undefined
  const incomingVariant = incoming.variant?.trim() || undefined
  if (!incomingModel && incomingVariant === undefined) {
    return false
  }
  const currentModel = run.model?.trim() || undefined
  const currentVariant = run.variant?.trim() || undefined
  if (incomingModel === currentModel && incomingVariant === currentVariant) {
    return false
  }
  if (
    lastNativeSelection
    && incomingModel === (lastNativeSelection.model?.trim() || undefined)
    && incomingVariant === (lastNativeSelection.variant?.trim() || undefined)
  ) {
    return false
  }
  return true
}

function compactSelection(selection: NativeProviderSelection): NativeProviderSelection {
  return {
    ...(selection.model ? { model: selection.model } : {}),
    ...(selection.variant ? { variant: selection.variant } : {}),
    ...(selection.agent ? { agent: selection.agent } : {}),
  }
}

async function readRequestJson<T>(request: IncomingMessage): Promise<T> {
  const body = await readRequestBuffer(request)
  if (body.length === 0) {
    return {} as T
  }
  return JSON.parse(body.toString("utf8")) as T
}

async function readRequestBuffer(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = []
  for await (const chunk of request) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
  }
  return Buffer.concat(chunks)
}

function startKernelPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
): { stop: () => void } {
  let stopped = false
  let inFlight = false
  const tick = async () => {
    if (stopped || inFlight) return
    inFlight = true
    try {
      await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
      await client.send<Record<string, unknown>>(pollRuntimeNoticesRequest(sessionId, attachmentId))
    } catch (error) {
      debugNativeMutation("pump_error", { error: formatError(error) })
    } finally {
      inFlight = false
    }
  }
  const interval = setInterval(() => {
    void tick()
  }, 250)
  void tick()
  return {
    stop: () => {
      stopped = true
      clearInterval(interval)
    },
  }
}

async function runOpenCodeAttach(options: {
  proxyUrl: string
  providerSessionId: string
  workingDirectory: string
}): Promise<void> {
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
  const args = [
    "attach",
    options.proxyUrl,
    "--session",
    options.providerSessionId,
    "--dir",
    options.workingDirectory,
  ]
  await new Promise<void>((resolve, reject) => {
    const child = spawn(executable, args, {
      stdio: "inherit",
      env: process.env,
    })
    child.once("error", reject)
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`opencode attach exited with ${signal ?? code}`))
    })
  })
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function debugNativeMutation(label: string, payload: unknown) {
  if (!process.env.ARROBA_OPENCODE_NATIVE_DEBUG) return
  const line = `[arroba opencode native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.ARROBA_OPENCODE_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}
