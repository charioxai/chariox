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
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type ApiSseApp = {
  post: (path: string, handler: (request: { body?: unknown }, reply: ApiSseReply) => unknown) => unknown
}

type ApiSseReply = {
  code: (code: number) => ApiSseReply
  headers: (headers: Record<string, string>) => ApiSseReply
  hijack: () => void
  raw: {
    destroyed?: boolean
    writeHead: (statusCode: number, headers: Record<string, string>) => unknown
    write: (chunk: string) => unknown
    end: () => unknown
  }
}

type StreamState = {
  started: boolean
  partialIds: Set<string>
}

export const API_SSE_INVOKE_PATH = "/invoke"

export function installApiSseJsonRoutes(
  app: ApiSseApp,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  app.post(API_SSE_INVOKE_PATH, async (request, reply) => {
    if (!isApiSseJsonPublication(publication)) {
      reply.code(404)
      return { error: "not found" }
    }
    try {
      validateInput(request.body ?? {}, publication.input_schema)
    } catch (error) {
      reply.code(400).headers({ "content-type": "application/json" })
      return { error: error instanceof Error ? error.message : String(error) }
    }

    const invocation: NormalizedInvocation = {
      publication_id: publication.publication_id,
      request_id: `api_${Date.now()}_${Math.random().toString(16).slice(2)}`,
      caller: { type: "anonymous", proof: { transport: "api_sse_json" } },
      input: request.body ?? {},
      mode: "async",
    }
    await streamApiSseInvocation(reply, publication, invocation, deps)
  })
}

export function isApiSseJsonPublication(publication: WorkflowPublicationConfig) {
  return publication.transport === "api_sse_json"
}

async function streamApiSseInvocation(
  reply: ApiSseReply,
  publication: WorkflowPublicationConfig,
  invocation: NormalizedInvocation,
  deps: GatewayDeps,
) {
  reply.hijack()
  reply.raw.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
  })
  const state: StreamState = { started: false, partialIds: new Set() }
  try {
    writeSse(reply, "queued", { invocation_id: invocation.request_id })
    const result = deps.invokeWorkflow
      ? await deps.invokeWorkflow(invocation)
      : await invokeKernelWorkflow({ ...publication, mode: "async" }, invocation)
    await streamApiSseResult(reply, publication, result, state, {
      requestId: invocation.request_id,
      injectedWorkflowInvoker: Boolean(deps.invokeWorkflow),
    })
  } catch (error) {
    writeSse(reply, "error", { error: error instanceof Error ? error.message : String(error) })
  } finally {
    reply.raw.end()
  }
}

async function streamApiSseResult(
  reply: ApiSseReply,
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
  state: StreamState,
  options: {
    requestId: string
    injectedWorkflowInvoker: boolean
  },
) {
  if (result.queued) {
    writeSse(reply, "queued", { result: result.response ?? null })
    if (!options.injectedWorkflowInvoker) {
      await streamQueuedInvocationByRequestId(reply, publication, options.requestId, state)
    }
    return
  }
  const initialRun = result.workflow_run
  if (!initialRun?.id) {
    writeSse(reply, "final", {
      message: result.response ?? null,
      artifacts: [],
      workflow_run: null,
    })
    return
  }
  emitWorkflowRunEvents(reply, initialRun, state)
  if (isTerminalWorkflowRunStatus(initialRun.status) || options.injectedWorkflowInvoker) return
  await streamWorkflowRunUntilFinal(reply, publication, initialRun.id, state)
}

async function streamQueuedInvocationByRequestId(
  reply: ApiSseReply,
  publication: WorkflowPublicationConfig,
  requestId: string,
  state: StreamState,
) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const workflowRun = await waitForWorkflowRunByInvocationRequestId(client, publication, requestId, {
      shouldContinue: () => !reply.raw.destroyed,
    })
    if (!workflowRun) {
      writeSse(reply, "timeout", { invocation_id: requestId })
      return
    }
    emitWorkflowRunEvents(reply, workflowRun, state)
    if (!isTerminalWorkflowRunStatus(workflowRun.status)) {
      await streamWorkflowRunUntilFinal(reply, publication, workflowRun.id, state)
    }
  } finally {
    await client.close().catch(() => {})
  }
}

async function streamWorkflowRunUntilFinal(
  reply: ApiSseReply,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
  state: StreamState,
) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const timeoutMs = publication.sync_timeout_ms ?? 30_000
    const pollMs = publication.poll_ms ?? 500
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline && !reply.raw.destroyed) {
      await pumpPublicationRuntime(client, publication)
      const response = await client.send<Record<string, unknown>>(
        getWorkflowRunRequest(publication.session_id, workflowRunId),
      )
      const workflowRun = (response.WorkflowRun as { workflow_run?: WorkflowRun } | undefined)?.workflow_run ?? null
      if (workflowRun) {
        emitWorkflowRunEvents(reply, workflowRun, state)
        if (isTerminalWorkflowRunStatus(workflowRun.status)) return
      }
      await sleep(pollMs)
    }
    writeSse(reply, "timeout", { workflow_run_id: workflowRunId })
  } finally {
    await client.close().catch(() => {})
  }
}

function emitWorkflowRunEvents(reply: ApiSseReply, workflowRun: WorkflowRun, state: StreamState) {
  if (!state.started) {
    state.started = true
    writeSse(reply, "started", { workflow_run_id: workflowRun.id, workflow_run: workflowRun })
  }
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
  if (isTerminalWorkflowRunStatus(workflowRun.status)) {
    writeSse(reply, "final", {
      workflow_run_id: workflowRun.id,
      message: workflowRun.final_output?.message ?? "",
      artifacts: workflowRun.final_output?.artifacts ?? [],
      workflow_run: workflowRun,
    })
  }
}

function writeSse(reply: ApiSseReply, event: string, payload: unknown) {
  reply.raw.write(`event: ${event}\n`)
  reply.raw.write(`data: ${JSON.stringify(payload)}\n\n`)
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
