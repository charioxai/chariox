import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getWorkflowRunRequest } from "@arroba/kernel-client/ipc-requests"

import {
  defaultKernelEndpoint,
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import {
  publicationCallerForRequest,
  publicationInvocationCaller,
  type VerifiedPublicationCallerClaims,
} from "./publication-caller-claims.js"
import { normalizeFinalOutput } from "./publication-final-output.js"
import { validateInput } from "./publication-parser.js"
import { waitForWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
import { pumpPublicationRuntime } from "./publication-runtime-pump.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
} from "./publication-trace-events.js"
import { publicationWaitTimeoutMs } from "./publication-timeouts.js"
import type {
  GatewayDeps,
  NormalizedInvocation,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type McpApp = {
  post: unknown
  options: unknown
}

type McpPostHandler = (request: { body?: unknown }, reply: McpReply) => unknown
type McpOptionsHandler = (_request: unknown, reply: McpReply) => unknown

type McpReply = {
  code: (code: number) => McpReply
  headers: (headers: Record<string, string>) => McpReply
  hijack?: () => void
  raw?: Partial<{
    destroyed?: boolean
    writeHead: (statusCode: number, headers?: Record<string, string>) => void
    write: (chunk: string) => void
    end: (chunk?: string) => void
  }>
}

type JsonRpcRequest = {
  jsonrpc?: string
  id?: unknown
  method?: string
  params?: unknown
}

const JSON_RPC_VERSION = "2.0"
const MCP_PROTOCOL_VERSION = "2025-03-26"
export const PUBLICATION_MCP_PATH = "/"

export function mcpInvokePath(publication: WorkflowPublicationConfig) {
  return publication.route?.trim() || PUBLICATION_MCP_PATH
}

export function installPublicationMcpRoutes(app: McpApp, publication: WorkflowPublicationConfig, deps: GatewayDeps) {
  const invokePath = mcpInvokePath(publication)
  registerMcpOptions(app, invokePath, async (_request: unknown, reply: McpReply) => {
    if (!isMcpPublication(publication)) {
      reply.code(404)
      return { error: "not found" }
    }
    reply.code(204).headers(mcpCorsHeaders())
    return null
  })
  registerMcpPost(app, invokePath, async (request: { body?: unknown }, reply: McpReply) => {
    if (!isMcpPublication(publication)) {
      reply.code(404)
      return { error: "not found" }
    }
    reply.headers(mcpCorsHeaders())
    const caller = publicationCallerForRequest(request)

    const rpc = parseJsonRpcRequest(request.body)
    if (!rpc) return jsonRpcError(null, -32600, "invalid request")
    if (rpc.id === undefined || rpc.id === null) {
      reply.code(202)
      return null
    }

    try {
      if (rpc.method === "initialize") return initializeResponse(rpc)
      if (rpc.method === "tools/list") return toolsListResponse(rpc, publication)
      if (rpc.method === "resources/list") return jsonRpcResult(rpc.id, { resources: [] })
      if (rpc.method === "resources/templates/list") return jsonRpcResult(rpc.id, { resourceTemplates: [] })
      if (rpc.method === "prompts/list") return jsonRpcResult(rpc.id, { prompts: [] })
      if (rpc.method === "tools/call") {
        return await streamedToolsCallResponse(reply, rpc, publication, deps, caller)
      }
      return jsonRpcError(rpc.id, -32601, "method not found")
    } catch (error) {
      return jsonRpcError(rpc.id, -32000, error instanceof Error ? error.message : String(error))
    }
  })
}

function registerMcpPost(app: McpApp, path: string, handler: McpPostHandler) {
  const post = app.post as (path: string, handler: McpPostHandler) => unknown
  return post.call(app, path, handler)
}

function registerMcpOptions(app: McpApp, path: string, handler: McpOptionsHandler) {
  const options = app.options as (path: string, handler: McpOptionsHandler) => unknown
  return options.call(app, path, handler)
}

export function isMcpPublication(publication: WorkflowPublicationConfig) {
  return publication.transport === "mcp"
}

function parseJsonRpcRequest(body: unknown): JsonRpcRequest | null {
  if (!body || typeof body !== "object" || Array.isArray(body)) return null
  const request = body as JsonRpcRequest
  if (request.jsonrpc !== JSON_RPC_VERSION || typeof request.method !== "string") return null
  return request
}

function initializeResponse(request: JsonRpcRequest) {
  const params = request.params && typeof request.params === "object" ? request.params as Record<string, unknown> : {}
  return jsonRpcResult(request.id, {
    protocolVersion: typeof params.protocolVersion === "string" ? params.protocolVersion : MCP_PROTOCOL_VERSION,
    capabilities: {
      tools: { listChanged: false },
      resources: { subscribe: false, listChanged: false },
      prompts: { listChanged: false },
    },
    serverInfo: { name: "arroba-publication", version: "0.1.0" },
  })
}

function toolsListResponse(request: JsonRpcRequest, publication: WorkflowPublicationConfig) {
  return jsonRpcResult(request.id, {
    tools: [{
      name: publicationToolName(publication),
      description: `Invoke published workflow ${publication.publication_id}.`,
      inputSchema: publication.input_schema ?? {
        type: "object",
        properties: {
          prompt: { type: "string" },
        },
      },
    }],
  })
}

async function streamedToolsCallResponse(
  reply: McpReply,
  request: JsonRpcRequest,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
  caller: VerifiedPublicationCallerClaims | null,
) {
  const raw = reply.raw
  if (!reply.hijack || !raw?.writeHead || !raw.write || !raw.end) {
    return await toolsCallResponse(request, publication, deps, caller)
  }
  reply.hijack()
  raw.writeHead(200, {
    ...mcpCorsHeaders(),
    "content-type": "application/json; charset=utf-8",
  })
  const heartbeat = setInterval(() => {
    if (!raw.destroyed) raw.write?.(" \n")
  }, mcpKeepaliveMs(publication))
  try {
    const result = await toolsCallResponse(request, publication, deps, caller)
    raw.end(`${JSON.stringify(result)}\n`)
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    raw.end(`${JSON.stringify(jsonRpcError(request.id, -32000, message))}\n`)
  } finally {
    clearInterval(heartbeat)
  }
  return undefined
}

async function toolsCallResponse(
  request: JsonRpcRequest,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
  caller: VerifiedPublicationCallerClaims | null,
) {
  const params = request.params && typeof request.params === "object" ? request.params as Record<string, unknown> : {}
  const toolName = typeof params.name === "string" ? params.name : null
  if (toolName !== publicationToolName(publication)) return jsonRpcError(request.id, -32602, "unknown tool")
  const input = params.arguments ?? {}
  validateInput(input, publication.input_schema)
  const invocation: NormalizedInvocation = {
    publication_id: publication.publication_id,
    request_id: `mcp_${Date.now()}_${Math.random().toString(16).slice(2)}`,
    caller: publicationInvocationCaller(caller, { transport: "mcp", tool_name: toolName }),
    input,
    mode: "sync",
  }
  const result = deps.invokeWorkflow
    ? await deps.invokeWorkflow(invocation)
    : await invokeKernelWorkflow({ ...publication, mode: "sync" }, invocation)
  const finalResult = deps.invokeWorkflow
    ? result
    : await resolveQueuedOrRunningResult(publication, invocation.request_id, result)
  return jsonRpcResult(request.id, workflowResultToMcpToolResult(finalResult, publication))
}

function mcpKeepaliveMs(publication: WorkflowPublicationConfig) {
  const configured = (publication as { readonly mcp_keepalive_ms?: unknown }).mcp_keepalive_ms
  return typeof configured === "number" && Number.isFinite(configured) && configured > 0 ? configured : 15_000
}

async function resolveQueuedOrRunningResult(
  publication: WorkflowPublicationConfig,
  requestId: string,
  result: WorkflowInvocationResult,
): Promise<WorkflowInvocationResult> {
  if (!result.queued && (!result.workflow_run || isTerminalWorkflowRunStatus(result.workflow_run.status))) return result
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    let workflowRun = result.workflow_run ?? null
    if (!workflowRun && result.queued) {
      workflowRun = await waitForWorkflowRunByInvocationRequestId(client, publication, requestId)
    }
    if (!workflowRun) return result
    workflowRun = await waitForWorkflowRunFinal(client, publication, workflowRun.id)
    return { accepted: true, workflow_run: workflowRun }
  } finally {
    await client.close().catch(() => {})
  }
}

async function waitForWorkflowRunFinal(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
) {
  const timeoutMs = publicationWaitTimeoutMs(publication)
  const pollMs = publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  let latest: WorkflowRun | null = null
  while (Date.now() < deadline) {
    await pumpPublicationRuntime(client, publication)
    const response = await client.send<Record<string, unknown>>(getWorkflowRunRequest(publication.session_id, workflowRunId))
    latest = (response.WorkflowRun as { workflow_run?: WorkflowRun } | undefined)?.workflow_run ?? null
    if (latest && isTerminalWorkflowRunStatus(latest.status)) return latest
    await sleep(pollMs)
  }
  return latest ?? { id: workflowRunId, status: "unknown" }
}

function workflowResultToMcpToolResult(result: WorkflowInvocationResult, publication: WorkflowPublicationConfig) {
  const workflowRun = result.workflow_run ?? null
  const finalOutput = normalizeFinalOutput(workflowRun?.final_output)
  const message = finalOutput.text || (result.queued ? "workflow invocation queued" : "")
  const traces = workflowRun
    ? collectPublicationTraceEvents(publication, workflowRun, createPublicationTraceStreamState())
    : []
  return {
    content: [{ type: "text", text: message }],
    structuredContent: {
      accepted: result.accepted,
      queued: result.queued === true,
      workflow_run_id: workflowRun?.id ?? null,
      status: workflowRun?.status ?? null,
      message: finalOutput.message,
      artifacts: finalOutput.artifacts,
      traces,
    },
    isError: workflowRun ? !isTerminalWorkflowRunStatus(workflowRun.status) || workflowRun.status !== "Completed" : result.queued === true,
  }
}

function publicationToolName(publication: WorkflowPublicationConfig) {
  const safePublicationId = publication.publication_id.replace(/[^A-Za-z0-9_]/g, "_")
  return `invoke_${safePublicationId}`
}

function jsonRpcResult(id: unknown, result: unknown) {
  return { jsonrpc: JSON_RPC_VERSION, id, result }
}

function jsonRpcError(id: unknown, code: number, message: string) {
  return { jsonrpc: JSON_RPC_VERSION, id: id ?? null, error: { code, message } }
}

function mcpCorsHeaders() {
  return {
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "POST, OPTIONS",
    "access-control-allow-headers": "content-type, accept",
  }
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
