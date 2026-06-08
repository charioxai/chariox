import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getWorkflowRunRequest } from "@arroba/kernel-client/ipc-requests"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import {
  publicationForAgentAppInvocation,
  registerAgentAppWorkflowRunEffects,
} from "./publication-agent-app-effects.js"
import { releaseAgentAppReplicaInvocation } from "./publication-agent-app-replicas.js"
import { findWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
import { pumpPublicationRuntime } from "./publication-runtime-pump.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
  type PublicationTraceStreamState,
} from "./publication-trace-events.js"
import type {
  GatewayRequest,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import {
  PUBLICATION_VIEWER_FORM_INVOKE_PATH,
  publicationViewerResultPage,
} from "./publication-viewer.js"
import { publicationWaitTimeoutMs } from "./publication-timeouts.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type HumanHttpApp = {
  get: (path: string, handler: (request: { params?: unknown }, reply: HumanHttpReply) => unknown) => unknown
}

type HumanHttpReply = {
  code: (code: number) => HumanHttpReply
  type: (contentType: string) => HumanHttpReply
  hijack: () => void
  raw: {
    destroyed?: boolean
    writeHead: (statusCode: number, headers: Record<string, string>) => unknown
    write: (chunk: string) => unknown
    end: () => unknown
  }
}

type HumanHttpStreamState = {
  partialIds: Set<string>
  traces: PublicationTraceStreamState
}

export const HUMAN_HTTP_FORM_INVOKE_PATH = PUBLICATION_VIEWER_FORM_INVOKE_PATH

export function installHumanHttpRoutes(app: HumanHttpApp, publication: WorkflowPublicationConfig) {
  app.get("/.well-known/arroba/publication/runs/:workflowRunId/events", async (request, reply) => {
    const params = request.params as { workflowRunId?: string }
    const workflowRunId = params.workflowRunId
    if (!workflowRunId) {
      reply.code(400)
      return { error: "workflow run id is required" }
    }
    await streamWorkflowRunEvents(reply, publication, workflowRunId)
  })

  app.get("/.well-known/arroba/publication/invocations/:requestId/events", async (request, reply) => {
    const params = request.params as { requestId?: string }
    const requestId = params.requestId
    if (!requestId) {
      reply.code(400)
      return { error: "invocation request id is required" }
    }
    await streamInvocationEvents(reply, publication, requestId)
  })
}

export function shouldReturnHumanHtml(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
) {
  if (!isHumanHttpPublication(publication)) return false
  if (request.method.toUpperCase() !== "GET") return false
  const accept = request.headers.accept
  const values = Array.isArray(accept) ? accept : [accept]
  return values.some((value) => typeof value === "string" && value.includes("text/html"))
}

export function forwardHumanHttpResult(
  reply: Pick<HumanHttpReply, "code" | "type">,
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
  invocationRequestId?: string,
) {
  registerAgentAppWorkflowRunEffects(publication, result.workflow_run, invocationRequestId)
  reply.code(200).type("text/html; charset=utf-8")
  return publicationViewerResultPage(publication, result, invocationRequestId)
}

function isHumanHttpPublication(publication: WorkflowPublicationConfig) {
  return !publication.transport || publication.transport === "human_http"
}

async function streamInvocationEvents(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  requestId: string,
) {
  reply.hijack()
  reply.raw.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
  })
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    writeSse(reply, "queued", { invocation_id: requestId })
    const workflowRun = await waitForAgentAppWorkflowRunByInvocationRequestId(client, publication, requestId, {
      shouldContinue: () => !reply.raw.destroyed,
    })
    if (!workflowRun) {
      writeSse(reply, "timeout", { invocation_id: requestId })
      return
    }
    const runtimePublication = publicationForAgentAppInvocation(publication, requestId)
    writeSse(reply, "status", { workflow_run: workflowRun })
    const state: HumanHttpStreamState = { partialIds: new Set(), traces: createPublicationTraceStreamState() }
    emitPartialOutputs(reply, workflowRun, state)
    emitTraceOutputs(reply, publication, workflowRun, state)
    if (isTerminalWorkflowRunStatus(workflowRun.status)) {
      registerAgentAppWorkflowRunEffects(runtimePublication, workflowRun, requestId)
      releaseAgentAppReplicaInvocation(publication, requestId)
      writeSse(reply, "final", { workflow_run: workflowRun })
      return
    }
    await streamWorkflowRunEventsWithClient(reply, runtimePublication, workflowRun.id, client, state, requestId)
  } catch (error) {
    writeSse(reply, "error", { error: error instanceof Error ? error.message : String(error) })
  } finally {
    await client.close().catch(() => {})
    reply.raw.end()
  }
}

async function streamWorkflowRunEvents(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
) {
  reply.hijack()
  reply.raw.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
  })
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    await streamWorkflowRunEventsWithClient(reply, publication, workflowRunId, client, {
      partialIds: new Set(),
      traces: createPublicationTraceStreamState(),
    })
  } catch (error) {
    writeSse(reply, "error", { error: error instanceof Error ? error.message : String(error) })
  } finally {
    await client.close().catch(() => {})
    reply.raw.end()
  }
}

async function streamWorkflowRunEventsWithClient(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
  client: LocalIpcClient,
  state: HumanHttpStreamState,
  invocationRequestId?: string | null,
) {
  const timeoutMs = publicationWaitTimeoutMs(publication)
  const pollMs = publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  let lastStatus: string | null = null
  while (Date.now() < deadline && !reply.raw.destroyed) {
    await pumpPublicationRuntime(client, publication)
    const response = await client.send<Record<string, unknown>>(
      getWorkflowRunRequest(publication.session_id, workflowRunId),
    )
    const workflowRun = (response.WorkflowRun as { workflow_run?: WorkflowRun } | undefined)?.workflow_run ?? null
    if (workflowRun && workflowRun.status !== lastStatus) {
      lastStatus = workflowRun.status
      writeSse(reply, "status", { workflow_run: workflowRun })
    }
    if (workflowRun) {
      emitPartialOutputs(reply, workflowRun, state)
      emitTraceOutputs(reply, publication, workflowRun, state)
    }
    if (workflowRun && isTerminalWorkflowRunStatus(workflowRun.status)) {
      registerAgentAppWorkflowRunEffects(publication, workflowRun, invocationRequestId)
      releaseAgentAppReplicaInvocation(publication, invocationRequestId)
      writeSse(reply, "final", { workflow_run: workflowRun })
      return
    }
    await sleep(pollMs)
  }
  writeSse(reply, "timeout", { workflow_run_id: workflowRunId })
}

function emitTraceOutputs(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun,
  state: HumanHttpStreamState,
) {
  for (const trace of collectPublicationTraceEvents(publication, workflowRun, state.traces)) {
    writeSse(reply, "trace", trace)
  }
}

function emitPartialOutputs(
  reply: HumanHttpReply,
  workflowRun: WorkflowRun,
  state: HumanHttpStreamState,
) {
  for (const output of workflowRun.intermediate_outputs ?? []) {
    if (state.partialIds.has(output.id)) continue
    state.partialIds.add(output.id)
    writeSse(reply, "partial", {
      id: output.id,
      workflow_run_id: workflowRun.id,
      message: output.output.message,
      valid: output.valid,
      warning: output.warning ?? null,
    })
  }
}

function writeSse(reply: HumanHttpReply, event: string, payload: unknown) {
  reply.raw.write(`event: ${event}\n`)
  reply.raw.write(`data: ${JSON.stringify(payload)}\n\n`)
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForAgentAppWorkflowRunByInvocationRequestId(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  requestId: string,
  options: { timeoutMs?: number; pollMs?: number; shouldContinue?: () => boolean } = {},
): Promise<WorkflowRun | null> {
  const timeoutMs = options.timeoutMs ?? publicationWaitTimeoutMs(publication)
  const pollMs = options.pollMs ?? publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline && (options.shouldContinue?.() ?? true)) {
    const workflowRun = await findWorkflowRunByInvocationRequestId(
      client,
      publicationForAgentAppInvocation(publication, requestId),
      requestId,
    )
    if (workflowRun) return workflowRun
    await sleep(pollMs)
  }
  return null
}
