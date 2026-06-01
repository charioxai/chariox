import type { AgentInstance, SliceRecord } from "./cli-types.js"

export function formatAgentLabel(agent: AgentInstance | null | undefined) {
  if (!agent) {
    return ""
  }
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

export function formatAgentLocationLabel(
  agent: AgentInstance | null | undefined,
  slices: readonly SliceRecord[],
): string | null {
  const remote = agent?.remote_execution
  if (!remote) {
    return null
  }
  const slice = slices.find((candidate) => agent ? candidate.agent_ids?.includes(agent.id) : false)
    ?? slices.find((candidate) =>
      candidate.worker_kernel_id === remote.worker_kernel_id
      || candidate.worker_kernel_ref === remote.worker_kernel_id
      || candidate.worker_machine_id === remote.worker_machine_id,
    )
  if (slice) {
    return `slice:${slice.name || slice.id}`
  }
  return `remote:${remote.worker_kernel_id}`
}
