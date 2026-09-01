import type { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { listWorkflowRunsRequest } from "@chariox/kernel-client/ipc-requests"

import { lookupPublicationQueueDepth } from "./publication-status.js"
import type { WorkflowPublicationConfig } from "./publication-types.js"

export async function readConnectedPublicationOperationalStatus(
  client: Pick<LocalIpcClient, "send">,
  publication: WorkflowPublicationConfig,
): Promise<Record<string, unknown>> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      collectConnectedPublicationStatus(client, publication),
      new Promise<Record<string, unknown>>((resolve) => {
        timer = setTimeout(() => resolve({}), 5_000)
      }),
    ])
  } catch {
    // Registration can remain available without inventing a zero queue or
    // reporting stale run data when the owner's kernel cannot be inspected.
    return {}
  } finally {
    if (timer) clearTimeout(timer)
  }
}

async function collectConnectedPublicationStatus(
  client: Pick<LocalIpcClient, "send">,
  publication: WorkflowPublicationConfig,
): Promise<Record<string, unknown>> {
  const [queueDepth, response] = await Promise.all([
    lookupPublicationQueueDepth(client, publication),
    client.send<Record<string, unknown>>(
      listWorkflowRunsRequest(publication.session_id, publication.workflow_ref, { limit: 100 }),
    ),
  ])
  const runs = record(response.WorkflowRunsListed)?.workflow_runs
  if (!Array.isArray(runs)) return { queue_depth: queueDepth }
  return {
    queue_depth: queueDepth,
    recent_runs: runs.filter((value) => {
      const run = record(value)
      return run?.workflow_id === publication.workflow_ref
        && run.endpoint_id === publication.endpoint_ref
        && record(run.publication_invocation)?.publication_id === publication.publication_id
    }).sort((left, right) => runTime(right) - runTime(left)).slice(0, 5),
  }
}

function runTime(value: unknown): number {
  const time = record(value)?.created_at_ms
  return typeof time === "number" && Number.isFinite(time) ? time : 0
}

// Only operational scalars may leave the owner's kernel for Cloud. Never spread
// workflow runs or their invocation envelopes into a backend registration.
export function publicationCloudOperationalFields(value: unknown): Record<string, unknown> {
  const status = record(value)
  if (!status) return {}
  const queueDepth = status.queue_depth
  return {
    ...(typeof queueDepth === "number" && Number.isFinite(queueDepth) && queueDepth >= 0
      ? { queueDepth }
      : {}),
    ...(Array.isArray(status.recent_runs) ? {
      runs: status.recent_runs.flatMap((value) => {
        const run = record(value)
        if (!run || typeof run.id !== "string" || !run.id.trim()) return []
        const invocation = record(run.publication_invocation)
        return [{
          id: run.id,
          ...(typeof run.status === "string" ? { status: run.status } : {}),
          ...timestamp(run, "created_at_ms"),
          ...timestamp(run, "completed_at_ms"),
          ...(typeof invocation?.invocation_id === "string" ? {
            publication_invocation: { invocation_id: invocation.invocation_id },
          } : {}),
        }]
      }),
    } : {}),
  }
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function timestamp(run: Record<string, unknown>, key: string): Record<string, number> {
  const value = run[key]
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? { [key]: value } : {}
}
