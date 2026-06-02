export type ProviderRunRecoveryAgent = {
  readonly id: string
  readonly agent_ref: string
  readonly state?: string | null
  readonly is_processing?: boolean | null
  readonly remote_execution?: {
    readonly worker_machine_id?: string | null
    readonly active_worker_provider_run_id?: string | null
  } | null
}

export type ProviderRunRecoveryContext = {
  readonly agent: ProviderRunRecoveryAgent
  readonly activeProviderRunId?: string | null | undefined
  readonly activeProviderRunAgentId?: string | null | undefined
}

export function providerRunRecoveryActions(context: ProviderRunRecoveryContext): string[] {
  const agent = context.agent
  const sessionRunId = context.activeProviderRunId?.trim()
  const sessionRunAgentId = context.activeProviderRunAgentId?.trim()
  const workerRunId = agent.remote_execution?.active_worker_provider_run_id?.trim()
  if (sessionRunId && (!sessionRunAgentId || sessionRunAgentId !== agent.id)) {
    return [
      `run /kernel health and /provider processes; close or relaunch the mismatched provider run before sending more prompts to ${agent.agent_ref}`,
    ]
  }
  if (agent.remote_execution && (agent.state === "Working" || agent.is_processing) && !workerRunId) {
    const worker = agent.remote_execution.worker_machine_id?.trim() || "<worker-machine>"
    return [
      `run /kernel health and /machine kernels ${worker}; reconnect or relaunch the remote/slice worker if no active worker run appears`,
    ]
  }
  return []
}
