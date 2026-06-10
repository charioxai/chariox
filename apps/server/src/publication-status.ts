import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import {
  getWorkflowRunRequest,
  listWorkflowRunsRequest,
  listWorkflowWatchdogsRequest,
} from "@arroba/kernel-client/ipc-requests"
import type { WorkflowWatchdogDefinition } from "@arroba/kernel-client/kernel-types"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import { normalizeFinalOutput } from "./publication-final-output.js"
import { pumpPublicationRuntime } from "./publication-runtime-pump.js"
import { publicationHealthDetails } from "./publication-provider-readiness.js"
import type {
  GatewayDeps,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"

export async function publicationStatusPayload(
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const base = {
    ...basePublicationStatusPayload(publication),
    ...await publicationHealthDetails(publication, deps),
  }
  if (deps.getPublicationStatusDetails) {
    return {
      ...base,
      ...(await deps.getPublicationStatusDetails(publication)),
    }
  }
  if (deps.invokeWorkflow) return base
  return {
    ...base,
    ...(await lookupPublicationRuntimeStatus(publication)),
  }
}

function basePublicationStatusPayload(publication: WorkflowPublicationConfig) {
  return {
    status: "running",
    publication_id: publication.publication_id,
    runtime_session_id: publication.session_id,
    source_session_id: publication.source_session_id ?? null,
    workflow_ref: publication.workflow_ref,
    endpoint_ref: publication.endpoint_ref,
    hook_id: publication.hook_id ?? null,
    queue_ref: publication.queue_ref ?? "default",
    transport: publication.transport ?? "human_http",
    mode: publication.mode ?? "sync",
    route: publication.route ?? "/*",
    methods: publication.methods ?? ["GET", "POST"],
  }
}

async function lookupPublicationRuntimeStatus(publication: WorkflowPublicationConfig) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    await pumpPublicationRuntime(client, publication).catch(() => {})
    const watchdogs = await listEndpointWatchdogs(client, publication)
    const { latestRun, latestOutputRun } = await latestEndpointRunStatus(client, publication, watchdogs)
    return {
      runtime: { reachable: true },
      watchdog_count: watchdogs.length,
      watchdogs,
      latest_run: latestRun ? summarizeWorkflowRun(latestRun) : null,
      latest_output: latestWorkflowRunOutput(latestOutputRun ?? latestRun),
    }
  } catch (error) {
    return {
      runtime: {
        reachable: false,
        error: error instanceof Error ? error.message : String(error),
      },
      watchdog_count: null,
      watchdogs: [],
      latest_run: null,
      latest_output: null,
    }
  } finally {
    await client.close().catch(() => {})
  }
}

async function listEndpointWatchdogs(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
) {
  const response = await client.send<Record<string, unknown>>(
    listWorkflowWatchdogsRequest(publication.session_id, publication.workflow_ref),
  )
  const watchdogs = (response.WorkflowWatchdogsListed as { watchdogs?: WorkflowWatchdogDefinition[] } | undefined)?.watchdogs ?? []
  return watchdogs.filter((watchdog) => watchdog.endpoint_id === publication.endpoint_ref)
}

async function latestEndpointRunStatus(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  watchdogs: readonly WorkflowWatchdogDefinition[],
) {
  const latestWatchdogRunIds = new Set(
    watchdogs
      .map((watchdog) => watchdog.last_workflow_run_id)
      .filter((runId): runId is string => typeof runId === "string" && runId.length > 0),
  )
  const response = await client.send<Record<string, unknown>>(
    listWorkflowRunsRequest(publication.session_id, publication.workflow_ref),
  )
  const runs = (response.WorkflowRunsListed as { workflow_runs?: WorkflowRun[] } | undefined)?.workflow_runs ?? []
  const matchingRuns = runs.filter((run) => isPublicationEndpointRun(run, publication, latestWatchdogRunIds))
  const sortedRuns = matchingRuns.sort((left, right) => runSortKey(right) - runSortKey(left))
  const latestRun = sortedRuns[0] ? await detailedWorkflowRun(client, publication, sortedRuns[0]) : null
  let latestOutputRun = latestWorkflowRunOutput(latestRun) ? latestRun : null
  for (const run of sortedRuns.slice(latestOutputRun ? 1 : 0)) {
    const detailed = await detailedWorkflowRun(client, publication, run)
    if (latestWorkflowRunOutput(detailed)) {
      latestOutputRun = detailed
      break
    }
  }
  return { latestRun, latestOutputRun }
}

async function detailedWorkflowRun(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  run: WorkflowRun,
) {
  const detail = await client.send<Record<string, unknown>>(
    getWorkflowRunRequest(publication.session_id, run.id),
  )
  return (detail.WorkflowRun as { workflow_run?: WorkflowRun } | undefined)?.workflow_run ?? run
}

function isPublicationEndpointRun(
  run: WorkflowRun,
  publication: WorkflowPublicationConfig,
  latestWatchdogRunIds: ReadonlySet<string>,
) {
  if (run.workflow_id && run.workflow_id !== publication.workflow_ref) return false
  if (run.endpoint_id && run.endpoint_id !== publication.endpoint_ref) return false
  if (run.publication_invocation?.publication_id === publication.publication_id) return true
  if (latestWatchdogRunIds.has(run.id)) return true
  return !run.publication_invocation
}

function summarizeWorkflowRun(run: WorkflowRun) {
  return {
    id: run.id,
    status: run.status,
    workflow_id: run.workflow_id ?? null,
    endpoint_id: run.endpoint_id ?? null,
    created_at_ms: run.created_at_ms ?? null,
    completed_at_ms: run.completed_at_ms ?? null,
    publication_invocation: run.publication_invocation ?? null,
    final_output: run.final_output ?? null,
  }
}

function latestWorkflowRunOutput(run: WorkflowRun | null) {
  if (!run) return null
  if (run.final_output) {
    const finalOutput = normalizeFinalOutput(run.final_output)
    return {
      kind: "final",
      message: finalOutput.message,
      artifacts: finalOutput.artifacts,
    }
  }
  const partial = [...(run.intermediate_outputs ?? [])].sort((left, right) => (right.timestamp_ms ?? 0) - (left.timestamp_ms ?? 0))[0]
  if (!partial) return null
  return {
    kind: "partial",
    message: partial.output.message,
    artifacts: [],
    intermediate_output_id: partial.id,
  }
}

function runSortKey(run: WorkflowRun) {
  return run.created_at_ms ?? 0
}
