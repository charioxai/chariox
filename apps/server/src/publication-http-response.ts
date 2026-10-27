import type {
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
} from "./publication-types.js"
import { normalizeFinalOutput } from "./publication-final-output.js"
import { visibleWorkflowInvocationResult } from "./publication-workflow-run-visibility.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type GatewayReply = {
  code: (code: number) => unknown
  headers: (headers: Record<string, string>) => unknown
}

export function forwardWorkflowResult(
  reply: GatewayReply,
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
) {
  result = visibleWorkflowInvocationResult(publication, result)
  const workflowRun = result.workflow_run
  const finalMessage = workflowRun?.final_output ? normalizeFinalOutput(workflowRun.final_output).text : ""
  const transportResponse = parseTransportResponse(finalMessage)
  if (transportResponse) {
    reply.code(transportResponse.status)
    reply.headers(transportResponse.headers)
    return transportResponse.body ?? ""
  }
  if (result.queued) {
    reply.code(202)
    return { accepted: true, queued: true, result: result.response }
  }
  if (workflowRun && !isTerminalWorkflowRunStatus(workflowRun.status)) {
    reply.code(202)
    return { accepted: true, workflow_run: workflowRun }
  }
  reply.code(200)
  return {
    accepted: result.accepted,
    workflow_run: workflowRun ?? null,
    final_output: workflowRun?.final_output ?? null,
  }
}

function parseTransportResponse(message: string | undefined | null): { status: number; headers: Record<string, string>; body: unknown } | null {
  if (!message) return null
  try {
    const parsed = JSON.parse(message) as {
      kind?: string
      status?: number
      headers?: Record<string, string>
      body?: unknown
    }
    if (parsed.kind !== "http_response") return null
    return {
      status: parsed.status ?? 200,
      headers: parsed.headers ?? {},
      body: parsed.body ?? "",
    }
  } catch {
    return null
  }
}
