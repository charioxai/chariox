import http from "node:http"
import { setTimeout as sleep } from "node:timers/promises"

import WebSocket, { WebSocketServer } from "ws"

import {
  type RuntimeProviderRun,
  type TerminalOutputRecord,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { hiddenInstructionsStart, redactHiddenInstructionsFromJson } from "./hidden-instructions.js"
import { createCodexKernelOutputProjection } from "./codex-kernel-output-projection.js"
import {
  type CodexJsonRpcMessage,
  extractCodexThreadId,
  isCodexKernelInitialize,
  parseCodexJsonRpcMessage,
} from "./codex-json-rpc.js"
import { resolveCodexNativePermissionResponse } from "./codex-permission.js"
import {
  handleCodexNativeTurnStart,
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
  }>()
  let nextUpstreamRequestId = 1
  let upstreamSocket: WebSocket | null = null

  const ensureUpstream = () => {
    if (upstreamSocket && upstreamSocket.readyState !== WebSocket.CLOSED) return upstreamSocket
    const socket = new WebSocket(options.upstreamEndpoint)
    upstreamSocket = socket
    options.debug("upstream_connecting", { upstreamEndpoint: options.upstreamEndpoint })
    socket.on("open", () => options.debug("upstream_connected", { upstreamEndpoint: options.upstreamEndpoint }))
    socket.on("message", (raw) => handleUpstreamMessage(raw))
    socket.on("close", () => {
      options.debug("upstream_closed", { upstreamEndpoint: options.upstreamEndpoint })
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

  const kernelOutputProjection = createCodexKernelOutputProjection({
    agentId: options.agentId,
    broadcast: broadcastToNativeTuis,
    debug: options.debug,
  })

  const handleUpstreamMessage = (raw: WebSocket.RawData) => {
    const message = parseCodexJsonRpcMessage(raw)
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
          kernelOutputProjection.setThreadId(threadId)
          bindObservedThread(options, threadId)
        }
      }
      sendDownstream(pending.downstream, routedMessage)
      return
    }

    if (message.method === "thread/started") {
      const thread = message.params?.thread
      if (thread && typeof thread === "object" && "id" in thread && typeof thread.id === "string") {
        kernelOutputProjection.setThreadId(thread.id)
      }
    }
    broadcast(message)
  }

  const sendKernelInitializeResponse = (downstream: CodexDownstream, message: CodexJsonRpcMessage) => {
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
      if (message.method === "initialize") downstream.kind = "tui"
      if (message.method === "thread/start") {
        downstream.kind = "tui"
        forwardRequest(downstream, message)
        return
      }
      if (message.method === "turn/start" && downstream.kind !== "kernel") {
        void handleCodexNativeTurnStart(
          message,
          options,
          (response) => sendDownstream(downstream, response),
        )
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
  return Object.assign(server, { projectKernelOutputToTui: kernelOutputProjection.project })
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
