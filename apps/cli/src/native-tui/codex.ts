import { spawn, execFile } from "node:child_process"
import { appendFileSync } from "node:fs"
import http from "node:http"
import net from "node:net"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"

import WebSocket, { WebSocketServer } from "ws"

import {
  normalizeRuntimeSession,
  type AgentInstance,
  type RuntimeAttachment,
  type RuntimeProviderRun,
  type RuntimeSession,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  aliasAgentRequest,
  attachToSessionRequest,
  createSessionRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
  resolveSessionRequest,
  spawnAgentRequest,
  submitPromptRequest,
} from "../ipc-requests.js"

const execFileAsync = promisify(execFile)

type NativeCodexOptions = {
  sessionRef?: string
  socketPath?: string
  kernelUrl?: string
  kernelPort?: string
  clientId: string
  workspace?: string
  worktree?: string
  alias?: string
  agentAlias?: string
  model: string
  effort: string
  initialPrompt?: string
}

type JsonRpcMessage = {
  id?: unknown
  method?: string
  params?: Record<string, unknown>
  result?: Record<string, unknown>
  error?: unknown
}

type CodexDownstream = {
  socket: WebSocket
  kind: "unknown" | "tui" | "kernel"
}

export async function runCodexNativeTui(args: string[]): Promise<void> {
  const options = parseNativeCodexArgs(args)
  const inferredTargets = await inferWorkspaceTargetsFromLaunchDirectory(process.cwd())
  const workspace = options.workspace ?? inferredTargets.workspace
  const worktree = options.worktree ?? inferredTargets.worktree
  const client = new LocalIpcClient(options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint(options.kernelPort))
  let appServer: ReturnType<typeof spawn> | null = null
  let proxy: WebSocketServer | null = null
  let pump: { stop: () => void } | null = null

  try {
    const created = options.sessionRef
      ? null
      : await createSession(client, workspace, worktree, options.alias, options.model, options.effort)
    const session = created?.session ?? await resolveSession(client, options.sessionRef!, workspace)
    const attachment = await attachToSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await maybeAliasAgent(client, session.id, created.agent, options.agentAlias)
      : await spawnCodexAgent(client, session.id, options.agentAlias, options.model, options.effort, worktree)
    const upstreamEndpoint = `ws://127.0.0.1:${await reservePort()}`
    appServer = await startCodexAppServer(upstreamEndpoint, worktree)

    const bindState: {
      promise: Promise<RuntimeProviderRun> | null
      run: RuntimeProviderRun | null
      structuredEndpoint: string | null
    } = {
      promise: null,
      run: null,
      structuredEndpoint: null,
    }
    proxy = await startCodexProxy({
      upstreamEndpoint,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      model: options.model,
      effort: options.effort,
      bindState,
    })
    const proxyAddress = proxy.address()
    if (!proxyAddress || typeof proxyAddress === "string") {
      throw new Error("Codex proxy did not expose a TCP port")
    }
    const proxyUrl = `ws://127.0.0.1:${proxyAddress.port}`
    bindState.structuredEndpoint = proxyUrl
    process.stderr.write([
      "[arroba codex native-tui]",
      `  arroba session: ${session.id}${session.alias ? ` (${session.alias})` : ""}`,
      `  arroba agent:   ${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`,
      `  app-server:     ${upstreamEndpoint}`,
      `  proxy:          ${proxyUrl}`,
      "",
    ].join("\n"))
    pump = startKernelPumpLoop(client, session.id, attachment.id)
    await runCodexTui({
      proxyUrl,
      model: options.model,
      workingDirectory: worktree,
      initialPrompt: options.initialPrompt,
    })
  } finally {
    pump?.stop()
    if (proxy) {
      await new Promise<void>((resolve) => proxy!.close(() => resolve()))
    }
    if (appServer && appServer.exitCode == null) {
      appServer.kill("SIGTERM")
      await Promise.race([
        new Promise((resolve) => appServer?.once("exit", resolve)),
        sleep(2_000),
      ])
      if (appServer.exitCode == null) appServer.kill("SIGKILL")
    }
    await client.close()
  }
}

function parseNativeCodexArgs(args: string[]): NativeCodexOptions {
  const options: NativeCodexOptions = {
    clientId: `arroba-codex-native-${process.pid}`,
    model: "gpt-5.4-mini",
    effort: "high",
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
      case "--initial-prompt":
        options.initialPrompt = next()
        break
      case "--help":
      case "-h":
        printNativeCodexUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) throw new Error(`unknown codex argument ${arg}`)
        positional.push(arg)
    }
  }
  if (positional.length > 1) throw new Error("usage: arroba codex [session-ref]")
  if (options.kernelUrl && options.kernelPort) throw new Error("--kernel-url cannot be used together with --kernel-port")
  if (options.socketPath && options.kernelPort) throw new Error("--socket cannot be used together with --kernel-port")
  if (positional[0] !== undefined) options.sessionRef = positional[0]
  return options
}

function printNativeCodexUsage() {
  process.stdout.write([
    "usage: arroba codex [session-ref] [--socket PATH|--kernel-url URL|--kernel-port PORT]",
    "",
    "behavior:",
    "  creates a new Arroba agent in the selected session and launches native `codex --remote` for it.",
  ].join("\n") + "\n")
}

async function inferWorkspaceTargetsFromLaunchDirectory(cwd: string): Promise<{ workspace: string; worktree: string }> {
  try {
    const [worktreeResult, commonDirResult] = await Promise.all([
      execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd }),
      execFileAsync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd }),
    ])
    const worktree = worktreeResult.stdout.trim()
    const commonDir = commonDirResult.stdout.trim()
    if (!worktree) return { workspace: cwd, worktree: cwd }
    const workspace = commonDir.endsWith("/.git")
      ? commonDir.slice(0, -"/.git".length)
      : worktree
    return { workspace, worktree }
  } catch {
    return { workspace: cwd, worktree: cwd }
  }
}

function defaultKernelEndpoint(kernelPort?: string): string {
  if (process.env.ARROBA_KERNEL_URL) return process.env.ARROBA_KERNEL_URL
  const host = process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"
  const port = kernelPort ?? process.env.ARROBA_KERNEL_PORT ?? "43119"
  return `ws://${host}:${port}/kernel`
}

function parseKernelPort(value: string, arg: string): string {
  if (!/^\d+$/.test(value)) throw new Error(`${arg} must be a TCP port`)
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
  model = "gpt-5.4-mini",
  effort = "high",
): Promise<{ session: RuntimeSession; agent: AgentInstance | null }> {
  const response = await client.send<Record<string, unknown>>(
    createSessionRequest(workspace, worktree, alias, {
      provider: "codex",
      model,
      effort,
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

async function spawnCodexAgent(
  client: LocalIpcClient,
  sessionId: string,
  alias: string | undefined,
  model: string,
  effort: string,
  worktree: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    spawnAgentRequest(sessionId, "codex", alias, model, worktree, effort),
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

async function launchNativeProviderRun(options: {
  client: LocalIpcClient
  sessionId: string
  agentId: string
  model: string
  effort: string
  structuredEndpoint: string
  providerSessionId: string
}): Promise<RuntimeProviderRun> {
  const response = await options.client.send<Record<string, unknown>>(
    launchProviderRunRequest(
      options.sessionId,
      "codex",
      "default",
      options.model,
      options.effort,
      options.agentId,
      {
        structuredEndpoint: options.structuredEndpoint,
        providerSessionId: options.providerSessionId,
        nativeTui: true,
      },
    ),
  )
  const run = "ProviderRunLaunched" in response
    ? expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched").provider_run
    : expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunchAccepted").provider_run
  return waitForProviderRunReady(options.client, run.id)
}

async function waitForProviderRunReady(
  client: LocalIpcClient,
  providerRunId: string,
): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + 60_000
  let latest: RuntimeProviderRun | null = null
  while (Date.now() < deadline) {
    latest = expectVariant<{ provider_run: RuntimeProviderRun }>(
      await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId)),
      "ProviderRun",
    ).provider_run
    if (latest.state === "Running" && latest.provider_session_id) return latest
    if (latest.state === "Ended") throw new Error(`Codex provider run ended before attach was ready: ${providerRunId}`)
    await sleep(250)
  }
  throw new Error(`timed out waiting for Codex provider run to become ready: ${providerRunId} (${latest?.state ?? "unknown"})`)
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

async function startCodexAppServer(endpoint: string, workingDirectory: string) {
  const executable = process.env.ARROBA_CODEX_BIN?.trim() || "codex"
  const child = spawn(executable, ["app-server", "--listen", endpoint], {
    cwd: workingDirectory,
    stdio: ["ignore", "ignore", "inherit"],
    env: process.env,
  })
  child.once("error", (error) => {
    throw error
  })
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (await tcpEndpointIsReady(endpoint)) return child
    if (child.exitCode != null) throw new Error(`codex app-server exited before becoming ready: ${child.exitCode}`)
    await sleep(150)
  }
  throw new Error(`timed out waiting for codex app-server at ${endpoint}`)
}

async function tcpEndpointIsReady(endpoint: string): Promise<boolean> {
  const url = new URL(endpoint)
  return await new Promise((resolve) => {
    const socket = net.createConnection({
      host: url.hostname,
      port: Number(url.port),
    })
    const timer = setTimeout(() => {
      socket.destroy()
      resolve(false)
    }, 500)
    socket.once("connect", () => {
      clearTimeout(timer)
      socket.destroy()
      resolve(true)
    })
    socket.once("error", () => {
      clearTimeout(timer)
      resolve(false)
    })
  })
}

async function startCodexProxy(options: {
  upstreamEndpoint: string
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  model: string
  effort: string
  bindState: {
    promise: Promise<RuntimeProviderRun> | null
    run: RuntimeProviderRun | null
    structuredEndpoint: string | null
  }
}): Promise<WebSocketServer> {
  const httpServer = http.createServer((request, response) => {
    if (request.url === "/readyz") {
      response.writeHead(200, { "content-type": "text/plain" })
      response.end("ok\n")
      return
    }
    response.writeHead(404)
    response.end()
  })
  const server = new WebSocketServer({ server: httpServer })
  const downstreams = new Set<CodexDownstream>()
  const pendingRequests = new Map<unknown, {
    downstream: CodexDownstream
    originalId: unknown
    method: string | undefined
  }>()
  let nextUpstreamRequestId = 1
  let upstreamSocket: WebSocket | null = null

  const ensureUpstream = () => {
    if (upstreamSocket && upstreamSocket.readyState !== WebSocket.CLOSED) return upstreamSocket
    const socket = new WebSocket(options.upstreamEndpoint)
    upstreamSocket = socket
    debugNativeCodex("upstream_connecting", { upstreamEndpoint: options.upstreamEndpoint })
    socket.on("open", () => debugNativeCodex("upstream_connected", { upstreamEndpoint: options.upstreamEndpoint }))
    socket.on("message", (raw) => handleUpstreamMessage(raw))
    socket.on("close", () => {
      debugNativeCodex("upstream_closed", { upstreamEndpoint: options.upstreamEndpoint })
      for (const downstream of downstreams) downstream.socket.close()
    })
    socket.on("error", (error) => {
      debugNativeCodex("upstream_error", { error: error.message })
      broadcast({ method: "error", params: { message: error.message } })
    })
    return socket
  }

  const sendUpstream = (message: unknown) => {
    const socket = ensureUpstream()
    const payload = JSON.stringify(message)
    if (socket.readyState === WebSocket.OPEN) socket.send(payload)
    else socket.once("open", () => socket.send(payload))
  }

  const forwardRequest = (downstream: CodexDownstream, message: JsonRpcMessage) => {
    if (message.id === undefined) {
      sendUpstream(message)
      return
    }
    const upstreamId = `arroba-proxy-${nextUpstreamRequestId++}`
    pendingRequests.set(upstreamId, {
      downstream,
      originalId: message.id,
      method: message.method,
    })
    sendUpstream({ ...message, id: upstreamId })
  }

  const sendDownstream = (downstream: CodexDownstream, message: unknown) => {
    if (downstream.socket.readyState === WebSocket.OPEN) downstream.socket.send(JSON.stringify(message))
  }

  const broadcast = (message: unknown) => {
    for (const downstream of downstreams) sendDownstream(downstream, message)
  }

  const handleUpstreamMessage = (raw: WebSocket.RawData) => {
    const message = parseJsonRpcMessage(raw)
    if (!message) {
      for (const downstream of downstreams) sendRaw(downstream.socket, raw)
      return
    }

    if (message.id !== undefined && !message.method) {
      const pending = pendingRequests.get(message.id)
      if (!pending) {
        broadcast(message)
        return
      }
      pendingRequests.delete(message.id)
      const routedMessage = { ...message, id: pending.originalId }
      if (pending.method === "thread/start" && message.result) {
        const threadId = extractCodexThreadId(message)
        if (threadId) bindObservedThread(options, threadId)
      }
      sendDownstream(pending.downstream, routedMessage)
      return
    }

    broadcast(message)
  }

  const sendKernelInitializeResponse = (downstream: CodexDownstream, message: JsonRpcMessage) => {
    sendDownstream(downstream, {
      id: message.id,
      result: {
        server: "arroba-codex-native-proxy",
        version: "0.0.0-native-tui",
      },
    })
  }

  server.on("connection", (clientSocket) => {
    const downstream: CodexDownstream = { socket: clientSocket, kind: "unknown" }
    downstreams.add(downstream)
    clientSocket.on("message", (raw) => {
      const message = parseJsonRpcMessage(raw)
      if (!message) {
        sendUpstreamRaw(raw)
        return
      }
      if (message.method === "initialize" && isKernelInitialize(message)) {
        downstream.kind = "kernel"
        debugNativeCodex("kernel_connected", { agentId: options.agentId })
        sendKernelInitializeResponse(downstream, message)
        return
      }
      if (message.method === "initialized" && downstream.kind === "kernel") {
        return
      }
      if (message.method === "initialize") downstream.kind = "tui"
      if (message.method === "thread/start") {
        downstream.kind = "tui"
        forwardRequest(downstream, message)
        return
      }
      if (message.method === "turn/start" && downstream.kind !== "kernel") {
        void handleNativeTurnStart(message, options, (response) => sendDownstream(downstream, response))
        return
      }
      forwardRequest(downstream, message)
    })
    clientSocket.on("close", () => downstreams.delete(downstream))
  })

  const sendUpstreamRaw = (raw: WebSocket.RawData) => {
    const socket = ensureUpstream()
    if (socket.readyState === WebSocket.OPEN) socket.send(raw)
    else socket.once("open", () => socket.send(raw))
  }

  await new Promise<void>((resolve, reject) => {
    if (httpServer.address()) {
      resolve()
      return
    }
    httpServer.once("error", reject)
    httpServer.listen(0, "127.0.0.1", () => {
      httpServer.off("error", reject)
      resolve()
    })
  })
  const closeWebSocketServer = server.close.bind(server)
  server.close = ((callback?: (err?: Error) => void) => {
    upstreamSocket?.close()
    for (const downstream of downstreams) downstream.socket.close()
    closeWebSocketServer((error?: Error) => {
      httpServer.close((httpError?: Error) => callback?.(error ?? httpError))
    })
  }) as WebSocketServer["close"]
  return server
}

function bindObservedThread(
  options: {
    client: LocalIpcClient
    sessionId: string
    agentId: string
    model: string
    effort: string
    bindState: {
      promise: Promise<RuntimeProviderRun> | null
      run: RuntimeProviderRun | null
      structuredEndpoint: string | null
    }
  },
  threadId: string,
) {
  if (options.bindState.promise) return
  const structuredEndpoint = options.bindState.structuredEndpoint
  if (!structuredEndpoint) throw new Error("Codex proxy endpoint was not initialized before thread binding")
  debugNativeCodex("thread_observed", { threadId })
  options.bindState.promise = launchNativeProviderRun({
    client: options.client,
    sessionId: options.sessionId,
    agentId: options.agentId,
    model: options.model,
    effort: options.effort,
    structuredEndpoint,
    providerSessionId: threadId,
  }).then((run) => {
    options.bindState.run = run
    debugNativeCodex("provider_run_bound", {
      providerRunId: run.id,
      providerSessionId: run.provider_session_id,
      structuredEndpoint,
    })
    return run
  })
}

function isKernelInitialize(message: JsonRpcMessage): boolean {
  const clientInfo = message.params?.clientInfo
  if (!clientInfo || typeof clientInfo !== "object") return false
  const name = (clientInfo as Record<string, unknown>).name
  return typeof name === "string" && name.includes("arroba")
}

function parseJsonRpcMessage(raw: WebSocket.RawData): JsonRpcMessage | null {
  try {
    return JSON.parse(raw.toString()) as JsonRpcMessage
  } catch {
    return null
  }
}

function sendRaw(socket: WebSocket, raw: WebSocket.RawData) {
  if (socket.readyState === WebSocket.OPEN) socket.send(raw)
}

function extractCodexThreadId(message: JsonRpcMessage): string | null {
  const thread = message.result?.thread
  if (thread && typeof thread === "object" && "id" in thread && typeof thread.id === "string") {
    return thread.id
  }
  const id = message.result?.id
  return typeof id === "string" ? id : null
}

async function handleNativeTurnStart(
  message: JsonRpcMessage,
  options: {
    client: LocalIpcClient
    sessionId: string
    attachmentId: string
    agentId: string
    bindState: {
      promise: Promise<RuntimeProviderRun> | null
      run: RuntimeProviderRun | null
      structuredEndpoint?: string | null
    }
  },
  sendClient: (message: unknown) => void,
) {
  try {
  const bindPromise = await waitForNativeBinding(options.bindState)
  if (!bindPromise) {
    throw new Error("Codex thread is not bound to Arroba yet")
  }
    await bindPromise
    const prompt = extractCodexPrompt(message.params)
    await options.client.send<Record<string, unknown>>(
      submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, []),
    )
    const turnId = `arroba-native-${Date.now()}`
    sendClient({
      id: message.id,
      result: {
        turn: {
          id: turnId,
          items: [],
          itemsView: "notLoaded",
          status: "inProgress",
          error: null,
          startedAt: null,
          completedAt: null,
          durationMs: null,
        },
      },
    })
    debugNativeCodex("native_prompt_submitted", { agentId: options.agentId, prompt })
  } catch (error) {
    sendClient({
      id: message.id,
      error: {
        code: -32000,
        message: error instanceof Error ? error.message : String(error),
      },
    })
  }
}

async function waitForNativeBinding(bindState: {
  promise: Promise<RuntimeProviderRun> | null
}): Promise<RuntimeProviderRun | null> {
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (bindState.promise) return bindState.promise
    await sleep(100)
  }
  return bindState.promise
}

function extractCodexPrompt(params: Record<string, unknown> | undefined): string {
  const input = Array.isArray(params?.input) ? params.input : []
  const text = input.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    return record.type === "text" && typeof record.text === "string" ? [record.text] : []
  }).join("\n")
  return text.endsWith("\n") ? text : `${text}\n`
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
      debugNativeCodex("pump_error", { error: formatError(error) })
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

async function runCodexTui(options: {
  proxyUrl: string
  model: string
  workingDirectory: string
  initialPrompt?: string | undefined
}): Promise<void> {
  const executable = process.env.ARROBA_CODEX_BIN?.trim() || "codex"
  const args = [
    "--remote",
    options.proxyUrl,
    "--no-alt-screen",
    "-C",
    options.workingDirectory,
    "-m",
    options.model,
  ]
  if (options.initialPrompt) args.push(options.initialPrompt)
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
      reject(new Error(`codex exited with ${signal ?? code}`))
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

function debugNativeCodex(label: string, payload: unknown) {
  if (!process.env.ARROBA_CODEX_NATIVE_DEBUG) return
  const line = `[arroba codex native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.ARROBA_CODEX_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}
