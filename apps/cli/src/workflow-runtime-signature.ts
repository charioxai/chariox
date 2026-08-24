import type { RuntimeSession } from "./cli-types.js"

export function workflowRuntimeSignature(session: RuntimeSession): string {
  const endpoints = (session.workflows ?? []).flatMap((workflow) => (
    (workflow.endpoints ?? []).map((endpoint) => (
      [
        workflow.id,
        workflow.alias ?? null,
        workflow.revision,
        endpoint.id,
        endpoint.alias ?? null,
        endpoint.entry_node_id,
        endpoint.max_instances ?? 1,
      ]
    ))
  ))
  const agents = (session.agents ?? []).map((agent) => [
    agent.id,
    agent.agent_ref ?? null,
    agent.alias ?? null,
    agent.provider,
    agent.model ?? null,
    agent.effort ?? null,
  ])
  const runs = (session.workflow_runs ?? []).map((run) => `${run.id}:${run.status}`)
  const instances = (session.workflow_runtime_instances ?? []).map((instance) => (
    `${instance.id}:${instance.status}:${instance.active_run_id ?? "-"}`
  ))
  return JSON.stringify([endpoints, agents, runs, instances])
}
