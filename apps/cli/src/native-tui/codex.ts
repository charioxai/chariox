import { spawn } from "node:child_process"
import { appendFileSync } from "node:fs"
import http from "node:http"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import WebSocket, { WebSocketServer } from "ws"

import {
  type RuntimeProviderRun,
  type RuntimeSession,
  type TerminalOutputRecord,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  getSessionStateRequest,
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
  respondToInteractionRequest,
  submitPromptRequest,
} from "../ipc-requests.js"
import {
  preparePromptAttachmentsForSubmit,
  promptAttachmentTransferIsForced,
} from "../prompt-attachment-transfer.js"
import { grantNativeCapabilities } from "./capability-grants.js"
import { hiddenInstructionsStart, redactHiddenInstructionsFromJson } from "./hidden-instructions.js"
import {
  defaultKernelEndpoint,
  inferWorkspaceTargetsFromLaunchDirectory,
  parseKernelPort,
  parseNativeMode as parseMode,
  parseNativePermissions as parsePermissions,
  reserveLocalPort as reservePort,
} from "./launch-environment.js"
import {
  type CodexAppServerProcess,
  releaseKernelPortLocks,
  reserveCodexKernelServerPort,
  startCodexAppServer,
  startCodexAppServerInKernel,
  stopCodexAppServerInKernel,
} from "./codex-app-server.js"
import { extractCodexAttachments, extractCodexPrompt } from "./codex-prompt.js"
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

type NativeCodexOptions = {
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
  model: string
  effort: string
  mode: "build" | "plan"
  permissions: "required" | "yolo"
  initialPrompt?: string
  serverInKernel: boolean
  grantMcps: string[]
  grantSkills: string[]
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

type CodexProxyServer = WebSocketServer & {
  projectKernelOutputToTui: (records: TerminalOutputRecord[]) => void
}

export async function runCodexNativeTui(args: string[]): Promise<void> {
  const options = parseNativeCodexArgs(args)
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
  let appServer: CodexAppServerProcess | null = null
  let kernelServerPid: string | null = null
  let proxy: CodexProxyServer | null = null
  let endpointBridge: { close: () => Promise<void> } | null = null
  let pump: { stop: () => void } | null = null
  let cleanupSessionId: string | null = null
  let cleanupAttachmentId: string | null = null

  try {
    const remotePlacement = Boolean(options.machineRef || options.sliceRef)
    const created = options.sessionRef
      ? null
      : await createNativeSession(client, workspace, worktree, options.alias, {
        provider: "codex",
        model: options.model,
        effort: options.effort,
        execution_mode: options.mode,
        permission_level: options.permissions,
      }, options.sliceRef)
    const session = created?.session ?? await resolveNativeSession(client, options.sessionRef!, workspace)
    const attachment = await attachNativeSession(client, session.id, options.clientId)
    cleanupSessionId = session.id
    cleanupAttachmentId = attachment.id
    const agent = created?.agent
      ? await prepareCreatedNativeAgent(client, session.id, created.agent, options.agentAlias, options.machineRef)
      : await spawnNativeAgent(client, session.id, "codex", options.agentAlias, options.model, worktree, options.effort, options.mode, options.permissions, options.machineRef, options.sliceRef)
    await grantNativeCapabilities(client, workspace, agent.id, options.grantMcps, options.grantSkills)
    const bindState: {
      promise: Promise<RuntimeProviderRun> | null
      run: RuntimeProviderRun | null
      structuredEndpoint: string | null
    } = {
      promise: null,
      run: null,
      structuredEndpoint: null,
    }
    let providerSessionId: string | null = null
    let upstreamEndpoint: string
    let bindProviderEndpoint: string
    if (options.serverInKernel && remotePlacement) {
      const run = await launchManagedNativeProviderRun({
        client,
        sessionId: session.id,
        agentId: agent.id,
        model: options.model,
        effort: options.effort,
      })
      if (!run.structured_endpoint) {
        throw new Error("Codex managed native server did not expose an endpoint")
      }
      bindState.promise = Promise.resolve(run)
      bindState.run = run
      if (run.provider_session_id) {
        providerSessionId = run.provider_session_id
        debugNativeCodex("thread_observed", { threadId: run.provider_session_id })
      }
      upstreamEndpoint = run.structured_endpoint
      bindProviderEndpoint = ""
    } else if (options.serverInKernel) {
      const port = await reserveCodexKernelServerPort()
      upstreamEndpoint = `ws://127.0.0.1:${port}`
      const listenHost = process.env.ARROBA_CODEX_KERNEL_SERVER_BIND_HOST?.trim() || "127.0.0.1"
      const listenEndpoint = `ws://${listenHost}:${port}`
      kernelServerPid = await startCodexAppServerInKernel({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        endpoint: upstreamEndpoint,
        listenEndpoint,
        workingDirectory: worktree,
      })
      bindProviderEndpoint = process.env.ARROBA_CODEX_KERNEL_SERVER_PORT_RANGE ? upstreamEndpoint : ""
    } else {
      upstreamEndpoint = `ws://127.0.0.1:${await reservePort()}`
      appServer = await startCodexAppServer(upstreamEndpoint, worktree)
      bindProviderEndpoint = ""
    }
    const bridgedEndpoint = await bridgeRemoteNativeProviderEndpoint(upstreamEndpoint, "Codex")
    upstreamEndpoint = bridgedEndpoint.endpoint
    endpointBridge = bridgedEndpoint
    proxy = await startCodexProxy({
      upstreamEndpoint,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      model: options.model,
      effort: options.effort,
      bindState,
      inlineLocalAttachments: Boolean(options.relayUrl) || remotePlacement || promptAttachmentTransferIsForced(),
    })
    const proxyAddress = proxy.address()
    if (!proxyAddress || typeof proxyAddress === "string") {
      throw new Error("Codex proxy did not expose a TCP port")
    }
    const proxyUrl = `ws://127.0.0.1:${proxyAddress.port}`
    bindState.structuredEndpoint = bindProviderEndpoint || proxyUrl
    process.stderr.write([
      "[arroba codex native-tui]",
      `  arroba session: ${session.id}${session.alias ? ` (${session.alias})` : ""}`,
      `  arroba agent:   ${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`,
      ...(bindState.run ? [`  provider run:   ${bindState.run.id}`] : []),
      `  app-server:     ${upstreamEndpoint}`,
      `  proxy:          ${proxyUrl}`,
      ...(providerSessionId ? [`  codex thread:   ${providerSessionId}`] : []),
      "  prompt policy:  native prompts pass through; Arroba observes the session",
      "",
    ].join("\n"))
    pump = startKernelPumpLoop(client, session.id, attachment.id, remotePlacement
      ? (records) => proxy?.projectKernelOutputToTui(records)
      : undefined)
    await runCodexTui({
      proxyUrl,
      model: options.model,
      workingDirectory: worktree,
      providerSessionId: remotePlacement ? null : providerSessionId,
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
    if (kernelServerPid) {
      await stopCodexAppServerInKernel(client, cleanupSessionId, cleanupAttachmentId, kernelServerPid, worktree).catch(() => {})
    }
    await endpointBridge?.close()
    releaseKernelPortLocks()
    await client.close()
  }
}

function parseNativeCodexArgs(args: string[]): NativeCodexOptions {
  const options: NativeCodexOptions = {
    clientId: `arroba-codex-native-${process.pid}`,
    model: "gpt-5.4-mini",
    effort: "high",
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
      case "--model":
      case "-m":
        options.model = next()
        break
      case "--effort":
        options.effort = next()
        break
      case "--mode": {
        const value = parseMode(next())
        options.mode = value
        break
      }
      case "--permissions": {
        const value = parsePermissions(next())
        options.permissions = value
        break
      }
      case "--initial-prompt":
        options.initialPrompt = next()
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
        printNativeCodexUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) throw new Error(`unknown codex argument ${arg}`)
        positional.push(arg)
    }
  }
  if (positional.length > 1) throw new Error("usage: arroba codex [session-ref]")
  if (options.relayUrl && !options.relayToken) throw new Error("--relay-url requires --relay-token")
  if (options.relayUrl && !options.targetDaemonId && !options.targetDaemonAlias) {
    throw new Error("--relay-url requires --target-daemon-id or --target-daemon-alias")
  }
  if (options.relayUrl && (options.kernelUrl || options.socketPath)) {
    throw new Error("--relay-url cannot be used together with --kernel-url or --socket")
  }
  if (options.relayUrl && options.kernelPort) throw new Error("--relay-url cannot be used together with --kernel-port")
  if (options.kernelUrl && options.kernelPort) throw new Error("--kernel-url cannot be used together with --kernel-port")
  if (options.socketPath && options.kernelPort) throw new Error("--socket cannot be used together with --kernel-port")
  if (options.machineRef && options.sliceRef) {
    throw new Error("--machine and --slice cannot be used together")
  }
  if (options.machineRef && !options.serverInKernel) {
    throw new Error("--machine requires --server-in-kernel so the Codex app-server is launched by the worker kernel")
  }
  if (options.sliceRef && !options.serverInKernel) {
    throw new Error("--slice requires --server-in-kernel so the Codex app-server is launched by the slice worker kernel")
  }
  if (positional[0] !== undefined) options.sessionRef = positional[0]
  return options
}

function printNativeCodexUsage() {
  process.stdout.write([
    "usage: arroba codex [session-ref] [--socket PATH|--kernel-url URL|--kernel-port PORT] [--mode build|plan] [--permissions required|yolo]",
    "       arroba codex [session-ref] --relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)",
    "",
    "placement:",
    "  --machine, --kernel-ref REF       Run the Arroba agent/provider on a remote worker kernel",
    "  --slice REF                       Run the Arroba agent/provider on a home-managed slice worker",
    "",
    "behavior:",
    "  --grant-mcp NAME                Grant an installed Arroba MCP to the native agent before provider launch",
    "  --grant-skill NAME              Grant an installed Arroba skill to the native agent before provider launch",
    "  creates a new Arroba agent in the selected session and launches native `codex --remote` for it.",
  ].join("\n") + "\n")
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
  const run = await requestNativeProviderRunLaunch(options.client, {
    sessionId: options.sessionId,
    provider: "codex",
    model: options.model,
    effort: options.effort,
    agentId: options.agentId,
    native: {
      structuredEndpoint: options.structuredEndpoint,
      providerSessionId: options.providerSessionId,
    },
  })
  if (run.session_id !== options.sessionId) return run
  return waitForProviderRunReady(options.client, run.id)
}

async function launchManagedNativeProviderRun(options: {
  client: LocalIpcClient
  sessionId: string
  agentId: string
  model: string
  effort: string
}): Promise<RuntimeProviderRun> {
  return requestNativeProviderRunLaunch(options.client, {
    sessionId: options.sessionId,
    provider: "codex",
    model: options.model,
    effort: options.effort,
    agentId: options.agentId,
  })
}

async function waitForProviderRunReady(
  client: LocalIpcClient,
  providerRunId: string,
): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + 60_000
  let latest: RuntimeProviderRun | null = null
  while (Date.now() < deadline) {
    latest = await getNativeProviderRun(client, providerRunId)
    if (latest.state === "Running" && latest.provider_session_id) return latest
    if (latest.state === "Ended") throw new Error(`Codex provider run ended before attach was ready: ${providerRunId}`)
    await sleep(250)
  }
  throw new Error(`timed out waiting for Codex provider run to become ready: ${providerRunId} (${latest?.state ?? "unknown"})`)
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
  inlineLocalAttachments: boolean
}): Promise<CodexProxyServer> {
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
  let nextProjectedTurnId = 1
  let upstreamSocket: WebSocket | null = null
  let projectedThreadId: string | null = null
  const projectedItems = new Map<string, {
    key: string
    turnId: string
    itemId: string
    kind: "agentMessage" | "reasoning"
    text: string
    timer: NodeJS.Timeout | null
  }>()

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
    if (payload.includes(hiddenInstructionsStart)) {
      debugNativeCodex("hidden_instructions_forwarded", {
        method: typeof message === "object" && message && "method" in message ? (message as JsonRpcMessage).method : null,
      })
    }
    if (payload.includes("\"localImage\"") || payload.includes("\"image\"")) {
      debugNativeCodex("attachments_forwarded", {
        method: typeof message === "object" && message && "method" in message ? (message as JsonRpcMessage).method : null,
      })
    }
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

  const messageForDownstream = (downstream: CodexDownstream, message: unknown) =>
    downstream.kind === "kernel" ? message : redactHiddenInstructionsFromJson(message)

  const sendDownstream = (downstream: CodexDownstream, message: unknown) => {
    if (downstream.socket.readyState === WebSocket.OPEN) {
      downstream.socket.send(JSON.stringify(messageForDownstream(downstream, message)))
    }
  }

  const broadcast = (message: unknown) => {
    for (const downstream of downstreams) sendDownstream(downstream, message)
  }

  const broadcastToNativeTuis = (message: unknown) => {
    for (const downstream of downstreams) {
      if (downstream.kind === "tui") sendDownstream(downstream, message)
    }
  }

  const turnPayload = (turnId: string, status: "inProgress" | "completed") => ({
    id: turnId,
    items: [],
    itemsView: "notLoaded",
    status,
    error: null,
    startedAt: Math.floor(Date.now() / 1000),
    completedAt: status === "completed" ? Math.floor(Date.now() / 1000) : null,
    durationMs: null,
  })

  const startProjectedTurn = () => {
    if (!projectedThreadId) return null
    const turnId = `arroba-projected-turn-${nextProjectedTurnId++}`
    broadcastToNativeTuis({
      jsonrpc: "2.0",
      method: "thread/status/changed",
      params: {
        threadId: projectedThreadId,
        status: { type: "active", activeFlags: [] },
      },
    })
    broadcastToNativeTuis({
      jsonrpc: "2.0",
      method: "turn/started",
      params: {
        threadId: projectedThreadId,
        turn: turnPayload(turnId, "inProgress"),
      },
    })
    return turnId
  }

  const completeProjectedItemSoon = (projection: {
    key: string
    turnId: string
    itemId: string
    kind: "agentMessage" | "reasoning"
    text: string
    timer: NodeJS.Timeout | null
  }) => {
    if (projection.timer) clearTimeout(projection.timer)
    projection.timer = setTimeout(() => {
      if (!projectedThreadId) return
      broadcastToNativeTuis({
        jsonrpc: "2.0",
        method: "item/completed",
        params: {
          item: projection.kind === "reasoning"
            ? { type: "reasoning", id: projection.itemId, summary: [], content: [] }
            : { type: "agentMessage", id: projection.itemId, text: projection.text, phase: "final_answer", memoryCitation: null },
          threadId: projectedThreadId,
          turnId: projection.turnId,
          completedAtMs: Date.now(),
        },
      })
      broadcastToNativeTuis({
        jsonrpc: "2.0",
        method: "thread/status/changed",
        params: {
          threadId: projectedThreadId,
          status: { type: "idle" },
        },
      })
      broadcastToNativeTuis({
        jsonrpc: "2.0",
        method: "turn/completed",
        params: {
          threadId: projectedThreadId,
          turn: turnPayload(projection.turnId, "completed"),
        },
      })
      projectedItems.delete(projection.key)
    }, 750)
  }

  const projectKernelOutputToTui = (records: TerminalOutputRecord[]) => {
    for (const record of records) {
      if (!projectedThreadId) continue
      if (record.agent_id && record.agent_id !== options.agentId) continue
      if (
        record.kind !== "prompt_echo"
        && record.kind !== "provider_output"
        && record.kind !== "provider_reasoning"
        && record.kind !== "provider_error"
      ) continue
      const delta = Buffer.from(record.bytes).toString("utf8")
      if (!delta) continue

      if (record.kind === "prompt_echo") {
        const turnId = startProjectedTurn()
        if (!turnId) continue
        const itemId = `arroba-projected-user-${Date.now()}-${nextProjectedTurnId}`
        broadcastToNativeTuis({
          jsonrpc: "2.0",
          method: "item/started",
          params: {
            item: {
              type: "userMessage",
              id: itemId,
              content: [{ type: "text", text: delta, text_elements: [] }],
            },
            threadId: projectedThreadId,
            turnId,
            startedAtMs: Date.now(),
          },
        })
        broadcastToNativeTuis({
          jsonrpc: "2.0",
          method: "item/completed",
          params: {
            item: {
              type: "userMessage",
              id: itemId,
              content: [{ type: "text", text: delta, text_elements: [] }],
            },
            threadId: projectedThreadId,
            turnId,
            completedAtMs: Date.now(),
          },
        })
        debugNativeCodex("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
        continue
      }

      const itemKind = record.kind === "provider_reasoning" ? "reasoning" : "agentMessage"
      const itemKey = `${itemKind}:${record.merge_key ?? "default"}`
      let projection = projectedItems.get(itemKey)
      if (!projection) {
        const turnId = startProjectedTurn()
        if (!turnId) continue
        const itemId = `arroba-projected-${itemKind}-${Date.now()}-${nextProjectedTurnId}`
        projection = { key: itemKey, turnId, itemId, kind: itemKind, text: "", timer: null }
        projectedItems.set(itemKey, projection)
        broadcastToNativeTuis({
          jsonrpc: "2.0",
          method: "item/started",
          params: {
            item: itemKind === "reasoning"
              ? { type: "reasoning", id: itemId, summary: [], content: [] }
              : { type: "agentMessage", id: itemId, text: "", phase: "final_answer", memoryCitation: null },
            threadId: projectedThreadId,
            turnId,
            startedAtMs: Date.now(),
          },
        })
      }
      projection.text += delta
      broadcastToNativeTuis({
        jsonrpc: "2.0",
        method: record.kind === "provider_reasoning" ? "item/reasoning/textDelta" : "item/agentMessage/delta",
        params: {
          threadId: projectedThreadId,
          turnId: projection.turnId,
          itemId: projection.itemId,
          delta,
        },
      })
      completeProjectedItemSoon(projection)
      debugNativeCodex("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
    }
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
        if (threadId) {
          projectedThreadId = threadId
          bindObservedThread(options, threadId)
        }
      }
      sendDownstream(pending.downstream, routedMessage)
      return
    }

    if (message.method === "thread/started") {
      const thread = message.params?.thread
      if (thread && typeof thread === "object" && "id" in thread && typeof thread.id === "string") {
        projectedThreadId = thread.id
      }
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
      if (message.id !== undefined && !message.method) {
        if (downstream.kind !== "kernel") {
          void resolveCodexNativePermissionResponse(message, options).then((resolved) => {
            if (!resolved) sendUpstream(message)
          }).catch((error) => {
            debugNativeCodex("native_permission_response_resolution_failed", { error: error instanceof Error ? error.message : String(error) })
            sendUpstream(message)
          })
          return
        }
        sendUpstream(message)
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
  return Object.assign(server, { projectKernelOutputToTui })
}

async function resolveCodexNativePermissionResponse(
  message: JsonRpcMessage,
  options: {
    client: LocalIpcClient
    sessionId: string
    agentId: string
  },
): Promise<boolean> {
  const choiceId = codexNativePermissionChoice(message)
  if (!choiceId) return false
  const response = await options.client.send<Record<string, unknown>>(getSessionStateRequest(options.sessionId))
  const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
  const interaction = session.active_interactions?.find((entry) =>
    entry.agent_id === options.agentId && entry.kind === "permission")
  if (!interaction) return false
  await options.client.send<Record<string, unknown>>(
    respondToInteractionRequest(options.sessionId, interaction.id, choiceId),
  )
  return true
}

function codexNativePermissionChoice(message: JsonRpcMessage): string | null {
  const result = message.result && typeof message.result === "object" ? message.result : null
  const decision = typeof result?.decision === "string" ? result.decision.toLowerCase() : ""
  const action = typeof result?.action === "string" ? result.action.toLowerCase() : ""
  const combined = `${decision} ${action}`
  if (combined.includes("decline") || combined.includes("deny") || combined.includes("reject")) {
    return "deny"
  }
  if (combined.includes("session") || combined.includes("always")) {
    return "allow_session"
  }
  if (combined.includes("accept") || combined.includes("allow")) {
    return "allow_once"
  }
  if (result && "permissions" in result) {
    return "allow_once"
  }
  return null
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
  debugNativeCodex("thread_observed", { threadId })
  if (options.bindState.promise) return
  const structuredEndpoint = options.bindState.structuredEndpoint
  if (!structuredEndpoint) throw new Error("Codex proxy endpoint was not initialized before thread binding")
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
    inlineLocalAttachments: boolean
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
    const attachments = await preparePromptAttachmentsForSubmit(extractCodexAttachments(message.params), {
      inlineLocalFiles: options.inlineLocalAttachments,
    })
    await options.client.send<Record<string, unknown>>(
      submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
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
    debugNativeCodex("native_prompt_submitted", { agentId: options.agentId, prompt, attachmentCount: attachments.length })
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

function startKernelPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  onTerminalRecords?: (records: TerminalOutputRecord[]) => void,
): { stop: () => void } {
  let stopped = false
  let inFlight = false
  const tick = async () => {
    if (stopped || inFlight) return
    inFlight = true
    try {
      const response = await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
      if (onTerminalRecords && "TerminalOutput" in response) {
        const records = (response.TerminalOutput as { records?: unknown[] }).records
        if (Array.isArray(records) && records.length > 0) {
          onTerminalRecords(records as TerminalOutputRecord[])
        }
      }
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
  providerSessionId?: string | null
  initialPrompt?: string | undefined
}): Promise<void> {
  const executable = process.env.ARROBA_CODEX_BIN?.trim() || "codex"
  const baseArgs = [
    "--remote",
    options.proxyUrl,
    "--no-alt-screen",
    "-C",
    options.workingDirectory,
    "-m",
    options.model,
  ]
  const args = options.providerSessionId
    ? ["resume", ...baseArgs, options.providerSessionId]
    : baseArgs
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
