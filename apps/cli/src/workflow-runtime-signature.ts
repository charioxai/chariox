import type { RuntimeSession } from "./cli-types.js"

export function workflowRuntimeSignature(session: RuntimeSession): string {
  const endpoints = (session.workflows ?? []).flatMap((workflow) => (
    (workflow.endpoints ?? []).map((endpoint) => (
      `${workflow.id}:${endpoint.id}:${endpoint.max_instances ?? 1}`
    ))
  ))
  const runs = (session.workflow_runs ?? []).map((run) => `${run.id}:${run.status}`)
  const instances = (session.workflow_runtime_instances ?? []).map((instance) => (
    `${instance.id}:${instance.status}:${instance.active_run_id ?? "-"}`
  ))
  return JSON.stringify([endpoints, runs, instances])
}
