import type {
  WorkflowEndpointRuntimeInstance,
  WorkflowRun,
} from "./cli-types.js"

export const WORKFLOW_ENDPOINT_DEFAULT_MAX_INSTANCES = 1
export const WORKFLOW_ENDPOINT_MAX_INSTANCES_LIMIT = 32

export type WorkflowEndpointPoolStatus = {
  capacity: number
  registered: number
  busyCount: number
  staleCount: number
  activeRunIds: string[]
}

export function workflowEndpointCapacity(endpoint: { id: string; max_instances?: number }): number {
  return endpoint.max_instances ?? WORKFLOW_ENDPOINT_DEFAULT_MAX_INSTANCES
}

export function parseWorkflowEndpointMaxInstances(value: string): number | null {
  if (!/^\d+$/.test(value)) {
    return null
  }
  const parsed = Number.parseInt(value, 10)
  if (!Number.isSafeInteger(parsed)
    || parsed < 1
    || parsed > WORKFLOW_ENDPOINT_MAX_INSTANCES_LIMIT) {
    return null
  }
  return parsed
}

export function buildWorkflowEndpointPoolStatus(
  workflowId: string,
  endpoint: { id: string; max_instances?: number },
  instances: readonly WorkflowEndpointRuntimeInstance[],
  runs: readonly WorkflowRun[],
): WorkflowEndpointPoolStatus {
  const endpointInstances = instances.filter((instance) => (
    instance.workflow_id === workflowId && instance.endpoint_id === endpoint.id
  ))
  const activeRunIds = new Set<string>()
  for (const instance of endpointInstances) {
    if (instance.active_run_id) {
      activeRunIds.add(instance.active_run_id)
    }
  }
  let busyCount = 0
  let staleCount = 0
  for (const instance of endpointInstances) {
    if (instance.status === "busy") {
      busyCount += 1
    }
    if (instance.status === "stale") {
      staleCount += 1
    }
  }
  for (const run of runs) {
    if (run.workflow_id === workflowId
      && run.endpoint_id === endpoint.id
      && !isTerminalWorkflowRunStatus(run.status)) {
      activeRunIds.add(run.id)
    }
  }
  return {
    capacity: workflowEndpointCapacity(endpoint),
    registered: endpointInstances.length,
    busyCount,
    staleCount,
    activeRunIds: [...activeRunIds],
  }
}

export function formatWorkflowEndpointPoolSummary(status: WorkflowEndpointPoolStatus): string {
  const parts = [
    `${status.busyCount}/${status.capacity} busy`,
    `${status.registered} registered`,
  ]
  if (status.staleCount > 0) {
    parts.push(`${status.staleCount} stale`)
  }
  parts.push(`${status.activeRunIds.length} active run${status.activeRunIds.length === 1 ? "" : "s"}`)
  return parts.join(" • ")
}

function isTerminalWorkflowRunStatus(status: string) {
  return status === "Completed" || status === "Failed" || status === "Stopped"
}
