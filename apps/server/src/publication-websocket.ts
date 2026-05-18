import type { IncomingMessage } from "node:http"
import type { Duplex } from "node:stream"

import { WebSocket as WsSocket, WebSocketServer, type RawData } from "ws"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getWorkflowRunRequest } from "@arroba/kernel-client/ipc-requests"

import {
  defaultKernelEndpoint,
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import { authenticateRequest } from "./publication-auth.js"
import { validateInput } from "./publication-parser.js"
import type {
  GatewayDeps,
  GatewayRequest,
  NormalizedInvocation,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type WebSocketUpgradeHost = {
  server: {
    on: (event: "upgrade", listener: (request: IncomingMessage, socket: Duplex, head: Buffer) => void) => unknown
  }
}

export function installPublicationWebSocket(
  app: WebSocketUpgradeHost,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const webSocketServer = new WebSocketServer({ noServer: true })
  app.server.on("upgrade", (request: IncomingMessage, socket: Duplex, head: Buffer) => {
    const pathname = new URL(request.url ?? "/", "http://127.0.0.1").pathname
    if (pathname !== "/.well-known/arroba/publication/ws") return
    webSocketServer.handleUpgrade(request, socket, head, (webSocket) => {
      webSocketServer.emit("connection", webSocket, request)
    })
  })
  webSocketServer.on("connection", (webSocket, request) => {
    void handlePublicationWebSocket(webSocket, request, publication, deps)
  })
  return webSocketServer
}

async function handlePublicationWebSocket(
  webSocket: WsSocket,
  request: IncomingMessage,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const gatewayRequest = gatewayRequestFromIncomingMessage(request)
  const auth = await authenticateRequest(gatewayRequest, publication, publication.auth ?? { mode: "anonymous" }, deps)
  if (!auth.ok) {
    sendWebSocketJson(webSocket, { type: "error", error: auth.message })
    webSocket.close(1008, auth.message)
    return
  }
  webSocket.on("message", (data) => {
    void handlePublicationWebSocketMessage(webSocket, data, publication, deps, auth.caller)
  })
  sendWebSocketJson(webSocket, { type: "ready", publication_id: publication.publication_id })
}

async function handlePublicationWebSocketMessage(
  webSocket: WsSocket,
  data: RawData,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
  caller: Record<string, unknown>,
) {
  try {
    const envelope = parseWebSocketEnvelope(data)
    if (envelope.type !== "invoke") {
      sendWebSocketJson(webSocket, { type: "error", error: "expected invoke message" })
      return
    }
    validateInput(envelope.input, publication.input_schema)
    const invocation: NormalizedInvocation = {
      publication_id: publication.publication_id,
      request_id: `ws_${Date.now()}_${Math.random().toString(16).slice(2)}`,
      caller,
      input: envelope.input,
      mode: publication.mode ?? "sync",
    }
    const result = deps.invokeWorkflow
      ? await deps.invokeWorkflow(invocation)
      : await invokeKernelWorkflow(publication, invocation)
    sendWebSocketJson(webSocket, { type: "accepted", ...result })
    if (!deps.invokeWorkflow && result.workflow_run?.id) {
      await streamWorkflowRun(webSocket, publication, result.workflow_run.id)
    } else if (result.workflow_run && isTerminalWorkflowRunStatus(result.workflow_run.status)) {
      sendWebSocketJson(webSocket, { type: "final", workflow_run: result.workflow_run })
    }
  } catch (error) {
    sendWebSocketJson(webSocket, { type: "error", error: error instanceof Error ? error.message : String(error) })
  }
}

async function streamWorkflowRun(webSocket: WsSocket, publication: WorkflowPublicationConfig, workflowRunId: string) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const timeoutMs = publication.sync_timeout_ms ?? 30_000
    const pollMs = publication.poll_ms ?? 500
    const deadline = Date.now() + timeoutMs
    let lastStatus: string | null = null
    while (Date.now() < deadline && webSocket.readyState === WsSocket.OPEN) {
      const response = await client.send<Record<string, unknown>>(
        getWorkflowRunRequest(publication.session_id, workflowRunId),
      )
      const workflowRun = (response.WorkflowRun as { workflow_run: WorkflowRun } | undefined)?.workflow_run ?? null
      if (workflowRun && workflowRun.status !== lastStatus) {
        lastStatus = workflowRun.status
        sendWebSocketJson(webSocket, { type: "status", workflow_run: workflowRun })
      }
      if (workflowRun && isTerminalWorkflowRunStatus(workflowRun.status)) {
        sendWebSocketJson(webSocket, { type: "final", workflow_run: workflowRun })
        return
      }
      await sleep(pollMs)
    }
    sendWebSocketJson(webSocket, { type: "timeout", workflow_run_id: workflowRunId })
  } finally {
    await client.close().catch(() => {})
  }
}

function gatewayRequestFromIncomingMessage(request: IncomingMessage): GatewayRequest {
  return {
    method: request.method ?? "GET",
    url: request.url ?? "/",
    headers: request.headers as Record<string, string | string[] | undefined>,
    query: Object.fromEntries(new URL(request.url ?? "/", "http://127.0.0.1").searchParams),
    raw: request,
  }
}

function parseWebSocketEnvelope(data: RawData): { type: string; input: unknown } {
  const raw = Array.isArray(data) ? Buffer.concat(data).toString("utf8") : Buffer.from(data as Buffer).toString("utf8")
  const parsed = JSON.parse(raw) as { type?: unknown; input?: unknown; payload?: unknown }
  return {
    type: typeof parsed.type === "string" ? parsed.type : "",
    input: parsed.input ?? parsed.payload ?? {},
  }
}

function sendWebSocketJson(webSocket: WsSocket, payload: unknown) {
  if (webSocket.readyState === WsSocket.OPEN) {
    webSocket.send(JSON.stringify(payload))
  }
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
