import { loadCodexPromptSettlements } from "./codex-prompt-settlement.js"
import http from "node:http"
import { setTimeout as sleep } from "node:timers/promises"

import WebSocket, { WebSocketServer } from "ws"

import {
  type AgentRuntimeActivity,
  type RuntimeProviderRun,
  type RuntimeInteraction,
  type TerminalOutputRecord,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { hiddenInstructionsStart, redactHiddenInstructionsFromJson } from "./hidden-instructions.js"
import { createCodexKernelPermissionProjection, isCodexKernelPermissionResponse } from "./codex-kernel-permission-projection.js"
import { createCodexKernelOutputProjection } from "./codex-kernel-output-projection.js"
import { bindCodexDisplayTurn, handleCodexNativeTurnSteer, mapCodexNativeCompaction } from "./codex-conversation-controls.js"
import {
  type CodexJsonRpcMessage,
  extractCodexThreadId,
  isCodexKernelInitialize,
  parseCodexJsonRpcMessage,
} from "./codex-json-rpc.js"
import { resolveCodexNativePermissionResponse } from "./codex-permission.js"
import {
  handleCodexNativeTurnStart,
  handleCodexNativeTurnInterrupt,
  type CodexNativeBindingState,
} from "./codex-turn-submission.js"
import {
  getNativeProviderRun,
  requestNativeProviderRunLaunch,
} from "./provider-run-control.js"

type CodexProxyDebug = (label: string, payload: unknown) => void

type CodexDownstream = {
  socket: WebSocket
  kind: "unknown" | "tui" | "kernel"
  compaction?: { managedThreadId: string; displayThreadId: string; turnId?: string }
  compactionPreparing?: boolean
}

type CodexProxyOptions = {
  upstreamEndpoint: string
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  model: string
  effort: string
  bindState: CodexNativeBindingState
  inlineLocalAttachments: boolean
  debug: CodexProxyDebug
}

export type CodexProxyServer = WebSocketServer & {
  projectKernelOutputToTui: (records: TerminalOutputRecord[]) => void
  projectKernelActivityToTui: (activity: AgentRuntimeActivity | undefined) => Promise<void>
  projectKernelInteractionsToTui: (interactions: RuntimeInteraction[]) => void
}

export function createCodexProjectionThreadTracker(options: {
  setThreadId: (threadId: string) => void
  debug: CodexProxyDebug
}) {
  let tuiThreadId: string | null = null
  let projectedThreadId: string | null = null
  const ephemeralThreadIds = new Set<string>()

  const setProjectedThreadId = (threadId: string) => {
    if (projectedThreadId === threadId) return
    projectedThreadId = threadId
    options.setThreadId(threadId)
  }

  return {
    bindTuiThread: (threadId: string, ephemeral = false) => {
      if (ephemeral) {
        ephemeralThreadIds.add(threadId)
        return
      }
      tuiThreadId = threadId
      setProjectedThreadId(threadId)
    },
    isEphemeralThread: (threadId: string) => ephemeralThreadIds.has(threadId),
    isDisplayThread: (threadId: unknown) => typeof threadId === "string" && threadId === tuiThreadId,
    isManagedThread: (threadId: string) => tuiThreadId !== null && threadId !== tuiThreadId && !ephemeralThreadIds.has(threadId),
    observeUpstreamThread: (threadId: string, ephemeral = false) => {
      if (ephemeral) ephemeralThreadIds.add(threadId)
      if (ephemeralThreadIds.has(threadId)) return
      if (tuiThreadId && threadId !== tuiThreadId) {
        options.debug("upstream_thread_started_ignored_for_projection", {
          tuiThreadId,
          upstreamThreadId: threadId,
        })
        return
      }
      setProjectedThreadId(threadId)
    },
  }
}

export async function startCodexProxy(options: CodexProxyOptions): Promise<CodexProxyServer> {
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
    ephemeral: boolean
  }>()
  let nextUpstreamRequestId = 1
  const internalRequests = new Map<unknown, (message: CodexJsonRpcMessage) => void>()
  let upstreamSocket: WebSocket | null = null

  const downstreamCounts = () => {
    let tui = 0
    let kernel = 0
    let unknown = 0
    for (const downstream of downstreams) {
      switch (downstream.kind) {
        case "tui":
          tui += 1
          break
        case "kernel":
          kernel += 1
          break
        default:
          unknown += 1
          break
      }
    }
    return { total: downstreams.size, tui, kernel, unknown }
  }

  const ensureUpstream = () => {
    if (upstreamSocket && upstreamSocket.readyState !== WebSocket.CLOSED) return upstreamSocket
    const socket = new WebSocket(options.upstreamEndpoint)
    upstreamSocket = socket
    options.debug("upstream_connecting", { upstreamEndpoint: options.upstreamEndpoint })
    socket.on("open", () => options.debug("upstream_connected", { upstreamEndpoint: options.upstreamEndpoint }))
    socket.on("message", (raw) => handleUpstreamMessage(raw))
    socket.on("close", () => {
      options.debug("upstream_closed", { upstreamEndpoint: options.upstreamEndpoint })
      for (const resolve of internalRequests.values()) resolve({ error: { code: -32000 } })
      for (const downstream of downstreams) downstream.socket.close()
    })
    socket.on("error", (error) => {
      options.debug("upstream_error", { error: error.message })
      broadcast({ method: "error", params: { message: error.message } })
    })
    return socket
  }

  const sendUpstream = (message: unknown) => {
    const socket = ensureUpstream()
    const payload = JSON.stringify(message)
    if (payload.includes(hiddenInstructionsStart)) {
      options.debug("hidden_instructions_forwarded", {
        method: typeof message === "object" && message && "method" in message ? (message as CodexJsonRpcMessage).method : null,
      })
    }
    if (payload.includes("\"collaborationMode\"")) {
      options.debug("collaboration_mode_forwarded", {
        method: typeof message === "object" && message && "method" in message ? (message as CodexJsonRpcMessage).method : null,
      })
    }
    if (payload.includes("\"localImage\"") || payload.includes("\"image\"")) {
      options.debug("attachments_forwarded", {
        method: typeof message === "object" && message && "method" in message ? (message as CodexJsonRpcMessage).method : null,
      })
    }
    if (socket.readyState === WebSocket.OPEN) socket.send(payload)
    else socket.once("open", () => socket.send(payload))
  }

  const forwardRequest = (downstream: CodexDownstream, message: CodexJsonRpcMessage) => {
    if (message.id === undefined) {
      sendUpstream(message)
      return
    }
    const upstreamId = `chariox-proxy-${nextUpstreamRequestId++}`
    pendingRequests.set(upstreamId, {
      downstream,
      originalId: message.id,
      method: message.method,
      ephemeral: message.params?.ephemeral === true,
    })
    sendUpstream({ ...message, id: upstreamId })
  }

  const subscribeManagedCompactionThread = async (threadId: string) => {
    const id = `chariox-proxy-control-${nextUpstreamRequestId++}`
    let timer: ReturnType<typeof setTimeout> | undefined
    try {
      const response = await new Promise<CodexJsonRpcMessage>((resolve, reject) => {
        internalRequests.set(id, resolve)
        timer = setTimeout(() => reject(new Error("managed Codex subscription timed out")), 15_000)
        // Rejoin an already-running thread with no policy overrides. Never
        // hydrate its hidden instructions/history into the display response.
        sendUpstream({ id, method: "thread/resume", params: { threadId, excludeTurns: true } })
      })
      if (response.error || extractCodexThreadId(response) !== threadId) throw new Error("managed Codex subscription was rejected")
    } finally {
      clearTimeout(timer)
      internalRequests.delete(id)
    }
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
    if (typeof message === "object" && message && "method" in message && message.method === "item/commandExecution/requestApproval") options.debug("native_kernel_permission_broadcast", { downstreams: downstreamCounts() })
    let sent = 0
    for (const downstream of downstreams) {
      if (downstream.kind === "tui") {
        sendDownstream(downstream, message)
        sent += 1
      }
    }
    if (sent === 0) options.debug("native_projection_no_tui_downstream", { agentId: options.agentId, downstreams: downstreamCounts() })
  }

  const kernelOutputProjection = createCodexKernelOutputProjection({
    agentId: options.agentId,
    broadcast: broadcastToNativeTuis,
    debug: options.debug,
    resolveSettledPrompts: ids => loadCodexPromptSettlements(options.client, options.sessionId, options.agentId, ids),
    onTurnMapped: (threadId, turnId, promptId) => bindCodexDisplayTurn(options.bindState, threadId, turnId, promptId),
  })
  const kernelPermissionProjection = createCodexKernelPermissionProjection({ ...options, broadcast: broadcastToNativeTuis, beforeProjectPrompt: kernelOutputProjection.finishPromptItemsForInteraction })
  const projectionThreadTracker = createCodexProjectionThreadTracker({
    setThreadId: kernelOutputProjection.setThreadId,
    debug: options.debug,
  })

  const handleUpstreamMessage = (raw: WebSocket.RawData) => {
    const message = parseCodexJsonRpcMessage(raw)
    if (!message) {
      for (const downstream of downstreams) sendRaw(downstream.socket, raw)
      return
    }
    if ([...downstreams].some(downstream => downstream.compaction)) options.debug("compaction_upstream_frame", { method: message.method, id: message.id, parameterKeys: Object.keys(message.params ?? {}), threadId: message.params?.threadId, conversationId: message.params?.conversationId, eventType: (message.params?.msg as { type?: unknown } | undefined)?.type, errorCode: (message.error as { code?: unknown } | undefined)?.code })

    if (message.id !== undefined && !message.method) {
      const internal = internalRequests.get(message.id)
      if (internal) {
        internal(message)
        return
      }
      if (typeof message.id === "string" && message.id.startsWith("chariox-proxy-control-")) return
      const pending = pendingRequests.get(message.id)
      if (!pending) {
        broadcast(message)
        return
      }
      pendingRequests.delete(message.id)
      if (pending.method === "thread/compact/start" && message.error) delete pending.downstream.compaction
      const routedMessage = { ...message, id: pending.originalId }
      if (pending.method === "thread/start" && pending.downstream.kind !== "kernel" && message.result) {
        const threadId = extractCodexThreadId(message)
        if (threadId) {
          projectionThreadTracker.bindTuiThread(threadId, pending.ephemeral)
          if (!pending.ephemeral) bindObservedThread(options, threadId)
        }
      }
      sendDownstream(pending.downstream, routedMessage)
      return
    }

    if (message.method === "thread/started") {
      const thread = message.params?.thread
      if (thread && typeof thread === "object" && "id" in thread && typeof thread.id === "string") {
        projectionThreadTracker.observeUpstreamThread(thread.id, "ephemeral" in thread && thread.ephemeral === true)
      }
    }
    const notificationThreadId = message.params?.threadId
      ?? message.params?.conversationId
      ?? (message.params?.thread as { id?: unknown } | undefined)?.id
    if (typeof notificationThreadId === "string" && projectionThreadTracker.isEphemeralThread(notificationThreadId)) {
      // The kernel provider client consumes a run's output stream. Utility
      // output would otherwise settle or overwrite the visible Room turn.
      for (const downstream of downstreams) {
        if (downstream.kind !== "kernel") sendDownstream(downstream, message)
      }
      return
    }
    // MP-08 / MP-10 / MP-11: The home prompt projection owns the visible
    // lifecycle. Execution-thread notifications must not overwrite its turn
    // identity; server requests still reach the native approval UI.
    if (message.id === undefined && typeof notificationThreadId === "string"
      && projectionThreadTracker.isManagedThread(notificationThreadId)) {
      for (const downstream of downstreams) {
        if (downstream.kind === "kernel") sendDownstream(downstream, message)
        // MP-08 / MP-10 / MP-11: An explicit native compaction controls the
        // managed conversation, but its own lifecycle belongs in the requesting
        // display. Keep this exception bounded to that control and its turn.
        const control = downstream.compaction
        if (!control || control.managedThreadId !== notificationThreadId) continue
        options.debug("managed_compaction_notification", { method: message.method, turnId: message.params?.turnId, turnIdFromTurn: (message.params?.turn as {id?:unknown} | undefined)?.id, itemType: (message.params?.item as { type?: unknown } | undefined)?.type })
        const turn = message.params?.turn as { id?: unknown } | undefined
        const turnId = message.params?.turnId ?? turn?.id
        const item = message.params?.item as { type?: unknown } | undefined
        if (message.method === "turn/started" && typeof turnId === "string" && !control.turnId) control.turnId = turnId
        if (typeof turnId !== "string" || turnId !== control.turnId) continue
        if (!["turn/started", "turn/completed", "thread/compacted", "error"].includes(message.method ?? "")
          && !(["item/started", "item/completed"].includes(message.method ?? "") && item?.type === "contextCompaction")) continue
        sendDownstream(downstream, { ...message, params: { ...message.params, threadId: control.displayThreadId } })
        if (message.method === "turn/completed") delete downstream.compaction
      }
      return
    }
    broadcast(message)
  }

  const sendKernelInitializeResponse = (downstream: CodexDownstream, message: CodexJsonRpcMessage) => {
    sendDownstream(downstream, {
      id: message.id,
      result: {
        server: "chariox-codex-native-proxy",
        version: "0.0.0-native-tui",
      },
    })
  }

  server.on("connection", (clientSocket) => {
    const downstream: CodexDownstream = { socket: clientSocket, kind: "unknown" }
    downstreams.add(downstream)
    options.debug("downstream_connected", { agentId: options.agentId, downstreams: downstreamCounts() })
    clientSocket.on("message", (raw) => {
      const message = parseCodexJsonRpcMessage(raw)
      if (!message) {
        sendUpstreamRaw(raw)
        return
      }
      if (message.method === "initialize" && isCodexKernelInitialize(message)) {
        downstream.kind = "kernel"
        options.debug("kernel_connected", { agentId: options.agentId })
        sendKernelInitializeResponse(downstream, message)
        return
      }
      if (message.method === "initialized" && downstream.kind === "kernel") {
        return
      }
      if (message.id !== undefined && !message.method) {
        if (downstream.kind !== "kernel") {
          if (isCodexKernelPermissionResponse(message)) {
            void kernelPermissionProjection.respond(message).catch(error => options.debug("native_kernel_permission_response_failed", { error: formatError(error) }))
            return
          }
          void resolveCodexNativePermissionResponse(message, options).then((resolved) => {
            if (!resolved) sendUpstream(message)
          }).catch((error) => {
            options.debug("native_permission_response_resolution_failed", { error: formatError(error) })
            sendUpstream(message)
          })
          return
        }
        sendUpstream(message)
        return
      }
      if (message.method === "initialize") {
        downstream.kind = "tui"
        options.debug("tui_connected", { agentId: options.agentId, downstreams: downstreamCounts() })
      }
      if (message.method === "thread/start") {
        if (downstream.kind !== "kernel") downstream.kind = "tui"
        options.debug("tui_thread_start", { agentId: options.agentId, downstreams: downstreamCounts() })
        forwardRequest(downstream, message)
        return
      }
      // Provider utility threads (for example automatic task titles) must not
      // become Room user prompts or replace the visible native thread.
      const threadId = message.params?.threadId
      const ephemeral = typeof threadId === "string" && projectionThreadTracker.isEphemeralThread(threadId)
      if (message.method === "turn/start" && downstream.kind !== "kernel" && !ephemeral) {
        void handleCodexNativeTurnStart(
          message,
          { ...options, onPromptStarted: kernelOutputProjection.startPrompt },
          (response) => sendDownstream(downstream, response),
        )
        return
      }
      if (message.method === "turn/interrupt" && downstream.kind !== "kernel" && !ephemeral) {
        void handleCodexNativeTurnInterrupt(message, options,
          (response) => sendDownstream(downstream, response))
        return
      }
      if (message.method === "turn/steer" && downstream.kind !== "kernel" && !ephemeral) {
        void handleCodexNativeTurnSteer(message, options, (response) => sendDownstream(downstream, response))
        return
      }
      if (message.method === "thread/compact/start" && downstream.kind !== "kernel" && !ephemeral) {
        if (!projectionThreadTracker.isDisplayThread(message.params?.threadId)) {
          sendDownstream(downstream, { id: message.id, error: { code: -32000, message: "Codex compaction must target the bound display thread" } })
          return
        }
        if (downstream.compaction || downstream.compactionPreparing) {
          sendDownstream(downstream, { id: message.id, error: { code: -32000, message: "managed Codex compaction is already active" } })
          return
        }
        downstream.compactionPreparing = true
        void mapCodexNativeCompaction(message, options).then(async (mapped) => {
          await subscribeManagedCompactionThread(mapped.params!.threadId as string)
          const fresh = await mapCodexNativeCompaction(message, options)
          if (fresh.params!.threadId !== mapped.params!.threadId) throw new Error("managed Codex conversation changed during subscription")
          downstream.compaction = { managedThreadId: mapped.params!.threadId as string, displayThreadId: message.params!.threadId as string }
          options.debug("managed_compaction_mapped", downstream.compaction)
          forwardRequest(downstream, mapped)
        }).catch((error) => {
          sendDownstream(downstream, { id: message.id, error: { code: -32000, message: formatError(error) } })
        }).finally(() => { delete downstream.compactionPreparing })
        return
      }
      forwardRequest(downstream, message)
    })
    clientSocket.on("close", () => {
      const kind = downstream.kind
      downstreams.delete(downstream)
      options.debug("downstream_closed", { agentId: options.agentId, kind, downstreams: downstreamCounts() })
    })
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
  return Object.assign(server, { projectKernelOutputToTui: kernelOutputProjection.project,
    projectKernelActivityToTui: kernelOutputProjection.projectAgentActivity,
    projectKernelInteractionsToTui: kernelPermissionProjection.project })
}

function bindObservedThread(options: CodexProxyOptions, threadId: string) {
  options.debug("thread_observed", { threadId })
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
    options.debug("provider_run_bound", {
      providerRunId: run.id,
      providerSessionId: run.provider_session_id,
      structuredEndpoint,
    })
    return run
  })
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

function sendRaw(socket: WebSocket, raw: WebSocket.RawData) {
  if (socket.readyState === WebSocket.OPEN) socket.send(raw)
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
