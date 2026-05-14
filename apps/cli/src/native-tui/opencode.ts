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
  normalizeRuntimeSession,
  type AgentInstance,
  type PromptAttachmentPart,
  type RuntimeAttachment,
  type RuntimeProviderRun,
  type RuntimeSession,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  aliasAgentRequest,
  attachToSessionRequest,
  cancelActivePromptRequest,
  createSessionRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
  resolveSessionRequest,
  spawnAgentRequest,
  submitPromptRequest,
  updateProviderRunSelectionRequest,
} from "../ipc-requests.js"
import { hiddenInstructionsStart, redactHiddenInstructions } from "./hidden-instructions.js"

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
  alias?: string
  agentAlias?: string
  mode: "build" | "plan"
  permissions: "required" | "yolo"
  serverInKernel: boolean
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
  let pump: { stop: () => void } | null = null
  try {
    const created = options.sessionRef
      ? null
      : await createSession(client, workspace, worktree, options.alias, options.mode, options.permissions)
    const session = created?.session ?? (options.sessionRef
      ? await resolveSession(client, options.sessionRef, workspace)
      : (() => {
        throw new Error("missing OpenCode session")
      })())
    const attachment = await attachToSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await maybeAliasAgent(client, session.id, created.agent, options.agentAlias)
      : await spawnOpenCodeAgent(client, session.id, options.agentAlias, options.mode, options.permissions)
    const proxyState: OpenCodeProxyState = {
      providerRunId: null,
      lastNativeSelection: null,
    }
    let upstreamBaseUrl: string
    let run: RuntimeProviderRun | null = null
    if (options.serverInKernel) {
      const launched = await launchProviderRun(client, session.id, "opencode", "default", "default", "", agent.id, {
        nativeTui: true,
      })
      run = await waitForOpenCodeRunReady(client, launched.id)
      if (!run.structured_endpoint) {
        throw new Error("OpenCode managed native server did not expose an endpoint")
      }
      upstreamBaseUrl = run.structured_endpoint
    } else {
      upstreamBaseUrl = `http://127.0.0.1:${await reservePort()}`
      openCodeServer = await startOpenCodeServer(upstreamBaseUrl, worktree)
    }
    proxy = await startOpenCodeProxy({
      upstreamBaseUrl,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
    }, proxyState)
    const proxyAddress = proxy.address()
    if (!proxyAddress || typeof proxyAddress === "string") {
      throw new Error("OpenCode proxy did not expose a TCP port")
    }
    const proxyUrl = `http://127.0.0.1:${proxyAddress.port}`
    if (!options.serverInKernel) {
      const launched = await launchProviderRun(client, session.id, "opencode", "default", "default", "", agent.id, {
        structuredEndpoint: proxyUrl,
        nativeTui: true,
      })
      run = await waitForOpenCodeRunReady(client, launched.id)
    }
    if (!run) {
      throw new Error("OpenCode provider run was not launched")
    }
    const providerSessionId = run.provider_session_id
    if (!providerSessionId) {
      throw new Error("OpenCode provider run did not expose provider_session_id")
    }
    proxyState.providerRunId = run.id
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
      workingDirectory: run.working_directory ?? session.worktree_id ?? worktree,
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
    await client.close()
  }
}

function parseNativeOpenCodeArgs(args: string[]): NativeOpenCodeOptions {
  const options: NativeOpenCodeOptions = {
    clientId: `arroba-opencode-native-${process.pid}`,
    mode: "build",
    permissions: "yolo",
    serverInKernel: false,
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
    "behavior:",
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

async function createSession(
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
  alias?: string,
  mode: "build" | "plan" = "build",
  permissions: "required" | "yolo" = "yolo",
): Promise<{ session: RuntimeSession; agent: AgentInstance | null }> {
  const response = await client.send<Record<string, unknown>>(
    createSessionRequest(workspace, worktree, alias, {
      provider: "opencode",
      model: "default",
      effort: null,
      execution_mode: mode,
      permission_level: permissions,
    }),
  )
  const payload = expectVariant<{ session: RuntimeSession; agent?: AgentInstance | null }>(response, "SessionCreated")
  return {
    session: normalizeRuntimeSession(payload.session),
    agent: payload.agent ?? null,
  }
}

async function resolveSession(client: LocalIpcClient, sessionRef: string, workspace: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(resolveSessionRequest(sessionRef, workspace))
  return normalizeRuntimeSession(expectVariant<{ session: RuntimeSession }>(response, "SessionResolved").session)
}

async function attachToSession(
  client: LocalIpcClient,
  sessionId: string,
  clientId: string,
): Promise<RuntimeAttachment> {
  const response = await client.send<Record<string, unknown>>(attachToSessionRequest(sessionId, clientId))
  return expectVariant<{ attachment: RuntimeAttachment }>(response, "SessionAttached").attachment
}

async function spawnOpenCodeAgent(
  client: LocalIpcClient,
  sessionId: string,
  alias?: string,
  mode: "build" | "plan" = "build",
  permissions: "required" | "yolo" = "yolo",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    spawnAgentRequest(sessionId, "opencode", alias, "default", undefined, null, mode, permissions),
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

async function launchProviderRun(
  client: LocalIpcClient,
  sessionId: string,
  provider: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId: string,
  native?: {
    structuredEndpoint?: string | null
    providerSessionId?: string | null
    nativeTui?: boolean | null
  },
): Promise<RuntimeProviderRun> {
  const nativeBinding = {
    ...(native?.structuredEndpoint !== undefined ? { structuredEndpoint: native.structuredEndpoint } : {}),
    ...(native?.providerSessionId !== undefined ? { providerSessionId: native.providerSessionId } : {}),
    nativeTui: true,
  }
  const response = await client.send<Record<string, unknown>>(
    launchProviderRunRequest(sessionId, provider, accountProfile, model, effort, agentId, nativeBinding),
  )
  const payload = "ProviderRunLaunched" in response
    ? expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched")
    : expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunchAccepted")
  return payload.provider_run
}

async function getProviderRun(client: LocalIpcClient, providerRunId: string): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId))
  return expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun").provider_run
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
    latest = await getProviderRun(client, providerRunId)
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
    const attachments = extractPromptAttachments(body)
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
    const run = await getProviderRun(options.client, state.providerRunId)
    if (selection && shouldApplyNativeSelection(selection, state.lastNativeSelection, run)) {
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
    } else if (selection) {
      debugNativeMutation("selection_ignored_as_stale", {
        incoming: selection,
        current: {
          model: run.model,
          variant: run.variant,
        },
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
    response.writeHead(409, { "content-type": "application/json" })
    response.end(JSON.stringify({
      error: "OpenCode permission responses are blocked for Arroba native TUI agents; answer provider permissions through Arroba.",
    }))
    return
  }

  await proxyToOpenCode(request, response, options.upstreamBaseUrl, !isKernelRequest)
}

async function proxyToOpenCode(
  request: IncomingMessage,
  response: ServerResponse,
  upstreamBaseUrl: string,
  redactForNativeTui = false,
): Promise<void> {
  const method = request.method ?? "GET"
  const target = new URL(request.url ?? "/", upstreamBaseUrl)
  const headers = new Headers()
  for (const [key, value] of Object.entries(request.headers)) {
    if (!value || key.toLowerCase() === "host" || key.toLowerCase() === "content-length") continue
    if (Array.isArray(value)) {
      for (const entry of value) headers.append(key, entry)
    } else {
      headers.set(key, value)
    }
  }
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
      cwd: options.workingDirectory,
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
