import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import {
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
} from "@arroba/kernel-client/ipc-requests"

import type {
  NormalizedInvocation,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

export function defaultKernelEndpoint() {
  return process.env.ARROBA_KERNEL_URL ?? `ws://${process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"}:${process.env.ARROBA_KERNEL_PORT ?? "43118"}`
}

export async function invokeKernelWorkflow(
  publication: WorkflowPublicationConfig,
  invocation: NormalizedInvocation,
): Promise<WorkflowInvocationResult> {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const prompt = JSON.stringify(invocation, null, 2)
    const response = await client.send<Record<string, unknown>>(invokeWorkflowEndpointRequest(
      publication.session_id,
      publication.workflow_ref,
      publication.endpoint_ref,
      prompt,
    ))
    if ("WorkflowPromptEnqueued" in response) {
      return { accepted: true, queued: true, response: response.WorkflowPromptEnqueued }
    }
    const invoked = response.WorkflowRunInvoked as { workflow_run: WorkflowRun } | undefined
    if (!invoked?.workflow_run) {
      throw new Error(`unexpected workflow invoke response: ${JSON.stringify(response)}`)
    }
    if ((publication.mode ?? "sync") === "async") {
      return { accepted: true, workflow_run: invoked.workflow_run }
    }
    const workflowRun = await waitForWorkflowRun(client, publication, invoked.workflow_run.id)
    return { accepted: true, workflow_run: workflowRun }
  } finally {
    await client.close().catch(() => {})
  }
}

async function waitForWorkflowRun(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
) {
  const timeoutMs = publication.sync_timeout_ms ?? 30_000
  const pollMs = publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  let latest: WorkflowRun | null = null
  while (Date.now() < deadline) {
    const response = await client.send<Record<string, unknown>>(
      getWorkflowRunRequest(publication.session_id, workflowRunId),
    )
    latest = (response.WorkflowRun as { workflow_run: WorkflowRun } | undefined)?.workflow_run ?? null
    if (latest && isTerminalWorkflowRunStatus(latest.status)) return latest
    await sleep(pollMs)
  }
  return latest ?? { id: workflowRunId, status: "unknown" }
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
