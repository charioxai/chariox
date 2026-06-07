import { listWorkflowRunsRequest } from "@arroba/kernel-client/ipc-requests"

import type {
  KernelLookupClient,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { publicationWaitTimeoutMs } from "./publication-timeouts.js"

export async function findWorkflowRunByInvocationRequestId(
  client: KernelLookupClient,
  publication: WorkflowPublicationConfig,
  requestId: string,
): Promise<WorkflowRun | null> {
  const response = await client.send(
    listWorkflowRunsRequest(publication.session_id, publication.workflow_ref),
  )
  const workflowRuns = (response.WorkflowRunsListed as { workflow_runs?: WorkflowRun[] } | undefined)?.workflow_runs ?? []
  return workflowRuns.find((workflowRun) => workflowRunMatchesInvocationRequestId(workflowRun, requestId)) ?? null
}

export async function waitForWorkflowRunByInvocationRequestId(
  client: KernelLookupClient,
  publication: WorkflowPublicationConfig,
  requestId: string,
  options: { timeoutMs?: number; pollMs?: number; shouldContinue?: () => boolean } = {},
): Promise<WorkflowRun | null> {
  const timeoutMs = options.timeoutMs ?? publicationWaitTimeoutMs(publication)
  const pollMs = options.pollMs ?? publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline && (options.shouldContinue?.() ?? true)) {
    const workflowRun = await findWorkflowRunByInvocationRequestId(client, publication, requestId)
    if (workflowRun) return workflowRun
    await sleep(pollMs)
  }
  return null
}

function workflowRunMatchesInvocationRequestId(workflowRun: WorkflowRun, requestId: string) {
  if (workflowRun.publication_invocation?.invocation_id === requestId) return true
  const prompt = workflowRun.invocation_prompt
  if (!prompt) return false
  try {
    const envelope = JSON.parse(prompt) as { request_id?: unknown }
    return envelope.request_id === requestId
  } catch {
    return false
  }
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
