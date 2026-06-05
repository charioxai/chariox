import type { IncomingMessage } from "node:http"
import type { Duplex } from "node:stream"

import { WebSocket as WsSocket, WebSocketServer, type RawData } from "ws"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getWorkflowRunRequest } from "@arroba/kernel-client/ipc-requests"

import {
  defaultKernelEndpoint,
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import { validateInput } from "./publication-parser.js"
import { waitForWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
import { pumpPublicationRuntime } from "./publication-runtime-pump.js"
import type {
  GatewayDeps,
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

type WebSocketArtifactUpload = {
  artifact_id: string
  name: string
  type: string
  size_bytes: number | null
  chunks: string[]
}

type WebSocketReadyArtifact = {
  artifact_id: string
  name: string
  type: string
  size_bytes: number | null
  base64: string
}

type WebSocketConnectionState = {
  pendingArtifacts: Map<string, WebSocketArtifactUpload>
  readyArtifacts: Map<string, WebSocketReadyArtifact>
  partialIds: Set<string>
  started: boolean
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
  _request: IncomingMessage,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const state: WebSocketConnectionState = {
    pendingArtifacts: new Map(),
    readyArtifacts: new Map(),
    partialIds: new Set(),
    started: false,
  }
  webSocket.on("message", (data) => {
    void handlePublicationWebSocketMessage(webSocket, data, publication, deps, { type: "anonymous" }, state)
  })
  sendWebSocketJson(webSocket, { type: "ready", publication_id: publication.publication_id })
}

async function handlePublicationWebSocketMessage(
  webSocket: WsSocket,
  data: RawData,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
  caller: Record<string, unknown>,
  state: WebSocketConnectionState,
) {
  try {
    const envelope = parseWebSocketEnvelope(data)
    if (handleWebSocketArtifactMessage(webSocket, envelope, state)) return
    if (envelope.type !== "invoke") {
      sendWebSocketJson(webSocket, { type: "error", error: "expected invoke message" })
      return
    }
    const input = attachWebSocketArtifacts(envelope.input, [...state.readyArtifacts.values()])
    validateInput(input, publication.input_schema)
    const invocation: NormalizedInvocation = {
      publication_id: publication.publication_id,
      request_id: `ws_${Date.now()}_${Math.random().toString(16).slice(2)}`,
      caller,
      input,
      mode: publication.mode ?? "sync",
    }
    state.readyArtifacts.clear()
    const result = deps.invokeWorkflow
      ? await deps.invokeWorkflow(invocation)
      : await invokeKernelWorkflow(publication, invocation)
    sendWebSocketJson(webSocket, { type: "accepted", ...result })
    sendWebSocketJson(webSocket, {
      type: "queued",
      invocation_id: invocation.request_id,
      result: result.queued ? result.response ?? null : null,
    })
    if (!deps.invokeWorkflow && result.workflow_run?.id) {
      await streamWorkflowRun(webSocket, publication, result.workflow_run.id, state)
    } else if (!deps.invokeWorkflow && result.queued) {
      await streamQueuedWorkflowRun(webSocket, publication, invocation.request_id, state)
    } else if (result.workflow_run && isTerminalWorkflowRunStatus(result.workflow_run.status)) {
      sendStarted(webSocket, result.workflow_run, state)
      sendPartialOutputs(webSocket, result.workflow_run, state)
      sendWebSocketJson(webSocket, { type: "final", workflow_run: result.workflow_run })
    }
  } catch (error) {
    sendWebSocketJson(webSocket, { type: "error", error: error instanceof Error ? error.message : String(error) })
  }
}

async function streamQueuedWorkflowRun(
  webSocket: WsSocket,
  publication: WorkflowPublicationConfig,
  requestId: string,
  state: WebSocketConnectionState,
) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const workflowRun = await waitForWorkflowRunByInvocationRequestId(client, publication, requestId, {
      shouldContinue: () => webSocket.readyState === WsSocket.OPEN,
    })
    if (!workflowRun) {
      sendWebSocketJson(webSocket, { type: "timeout", invocation_id: requestId })
      return
    }
    sendStarted(webSocket, workflowRun, state)
    sendWebSocketJson(webSocket, { type: "status", workflow_run: workflowRun })
    sendPartialOutputs(webSocket, workflowRun, state)
    if (isTerminalWorkflowRunStatus(workflowRun.status)) {
      sendWebSocketJson(webSocket, { type: "final", workflow_run: workflowRun })
      return
    }
    await streamWorkflowRun(webSocket, publication, workflowRun.id, state)
  } finally {
    await client.close().catch(() => {})
  }
}

async function streamWorkflowRun(
  webSocket: WsSocket,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
  state: WebSocketConnectionState,
) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const timeoutMs = publication.sync_timeout_ms ?? 30_000
    const pollMs = publication.poll_ms ?? 500
    const deadline = Date.now() + timeoutMs
    let lastStatus: string | null = null
    while (Date.now() < deadline && webSocket.readyState === WsSocket.OPEN) {
      await pumpPublicationRuntime(client, publication)
      const response = await client.send<Record<string, unknown>>(
        getWorkflowRunRequest(publication.session_id, workflowRunId),
      )
      const workflowRun = (response.WorkflowRun as { workflow_run: WorkflowRun } | undefined)?.workflow_run ?? null
      if (workflowRun) {
        sendStarted(webSocket, workflowRun, state)
      }
      if (workflowRun && workflowRun.status !== lastStatus) {
        lastStatus = workflowRun.status
        sendWebSocketJson(webSocket, { type: "status", workflow_run: workflowRun })
      }
      if (workflowRun) {
        sendPartialOutputs(webSocket, workflowRun, state)
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

function sendStarted(
  webSocket: WsSocket,
  workflowRun: WorkflowRun,
  state: WebSocketConnectionState,
) {
  if (state.started) return
  state.started = true
  sendWebSocketJson(webSocket, {
    type: "started",
    workflow_run_id: workflowRun.id,
    workflow_run: workflowRun,
  })
}

function sendPartialOutputs(
  webSocket: WsSocket,
  workflowRun: WorkflowRun,
  state: WebSocketConnectionState,
) {
  for (const output of workflowRun.intermediate_outputs ?? []) {
    if (state.partialIds.has(output.id)) continue
    state.partialIds.add(output.id)
    sendWebSocketJson(webSocket, {
      type: "partial",
      id: output.id,
      workflow_run_id: workflowRun.id,
      message: output.output.message,
      valid: output.valid,
      warning: output.warning ?? null,
    })
  }
}

function parseWebSocketEnvelope(data: RawData): Record<string, unknown> & { type: string; input: unknown } {
  const raw = Array.isArray(data) ? Buffer.concat(data).toString("utf8") : Buffer.from(data as Buffer).toString("utf8")
  const parsed = JSON.parse(raw) as Record<string, unknown> & { type?: unknown; input?: unknown; payload?: unknown }
  return {
    ...parsed,
    type: typeof parsed.type === "string" ? parsed.type : "",
    input: parsed.input ?? parsed.payload ?? {},
  }
}

function handleWebSocketArtifactMessage(
  webSocket: WsSocket,
  envelope: Record<string, unknown> & { type: string },
  state: WebSocketConnectionState,
) {
  if (envelope.type === "artifact_begin") {
    const artifactId = typeof envelope.artifact_id === "string" && envelope.artifact_id.trim()
      ? envelope.artifact_id
      : `artifact_${Date.now()}_${Math.random().toString(16).slice(2)}`
    state.pendingArtifacts.set(artifactId, {
      artifact_id: artifactId,
      name: typeof envelope.name === "string" && envelope.name.trim() ? envelope.name : artifactId,
      type: typeof envelope.mime_type === "string"
        ? envelope.mime_type
        : typeof envelope.type_hint === "string"
          ? envelope.type_hint
          : "application/octet-stream",
      size_bytes: typeof envelope.size_bytes === "number" ? envelope.size_bytes : null,
      chunks: [],
    })
    sendWebSocketJson(webSocket, { type: "artifact_ack", status: "begun", artifact_id: artifactId })
    return true
  }
  if (envelope.type === "artifact_chunk") {
    const artifactId = typeof envelope.artifact_id === "string" ? envelope.artifact_id : ""
    const upload = state.pendingArtifacts.get(artifactId)
    if (!upload || typeof envelope.data !== "string") {
      sendWebSocketJson(webSocket, { type: "error", error: "unknown artifact chunk" })
      return true
    }
    upload.chunks.push(envelope.data)
    sendWebSocketJson(webSocket, { type: "artifact_ack", status: "chunk", artifact_id: artifactId })
    return true
  }
  if (envelope.type === "artifact_end") {
    const artifactId = typeof envelope.artifact_id === "string" ? envelope.artifact_id : ""
    const upload = state.pendingArtifacts.get(artifactId)
    if (!upload) {
      sendWebSocketJson(webSocket, { type: "error", error: "unknown artifact end" })
      return true
    }
    state.pendingArtifacts.delete(artifactId)
    const artifact = {
      artifact_id: upload.artifact_id,
      name: upload.name,
      type: upload.type,
      size_bytes: upload.size_bytes,
      base64: upload.chunks.join(""),
    }
    state.readyArtifacts.set(artifactId, artifact)
    sendWebSocketJson(webSocket, { type: "artifact", status: "ready", artifact })
    return true
  }
  return false
}

function attachWebSocketArtifacts(input: unknown, artifacts: WebSocketReadyArtifact[]) {
  if (artifacts.length === 0) return input
  if (input && typeof input === "object" && !Array.isArray(input)) {
    const record = input as Record<string, unknown>
    const existing = Array.isArray(record.artifacts) ? record.artifacts : []
    return { ...record, artifacts: [...existing, ...artifacts] }
  }
  return { input, artifacts }
}

function sendWebSocketJson(webSocket: WsSocket, payload: unknown) {
  if (webSocket.readyState === WsSocket.OPEN) {
    webSocket.send(JSON.stringify(payload))
  }
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
