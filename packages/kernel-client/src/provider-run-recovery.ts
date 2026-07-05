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
  readonly agentBusy?: boolean | null | undefined
}

export function providerRunRecoveryActions(context: ProviderRunRecoveryContext): string[] {
  const agent = context.agent
  const sessionRunId = context.activeProviderRunId?.trim()
  const sessionRunAgentId = context.activeProviderRunAgentId?.trim()
  if (sessionRunId && (!sessionRunAgentId || sessionRunAgentId !== agent.id)) {
    return [
      `run /kernel health and /provider processes; export a debug bundle, then close or relaunch the mismatched provider run before sending more prompts to ${agent.agent_ref}`,
    ]
  }
  if (remoteWorkerProviderRunIsMissing(context)) {
    const worker = concreteRecoveryRef(agent.remote_execution?.worker_machine_id)
    return [
      worker
        ? `run /kernel remote-runtime and /machine kernels ${worker}; reconnect or relaunch the remote/slice worker before sending prompts to that remote/slice agent if no active worker run appears`
        : "run /kernel remote-runtime, identify the affected worker machine, then reconnect or relaunch the remote/slice worker before sending prompts to that remote/slice agent",
    ]
  }
  return []
}

export function remoteWorkerProviderRunIsMissing(context: Pick<ProviderRunRecoveryContext, "agent" | "agentBusy">): boolean {
  const agent = context.agent
  const workerRunId = agent.remote_execution?.active_worker_provider_run_id?.trim()
  const busy = context.agentBusy ?? (agent.state === "Working" || agent.is_processing)
  return Boolean(agent.remote_execution && busy && !workerRunId)
}

export function remoteWorkerProviderRunRecoveryAction(agentRef?: string | null, workerMachineId?: string | null): string {
  const agent = concreteRecoveryRef(agentRef)
  const worker = concreteRecoveryRef(workerMachineId)
  if (agent) {
    return `run /kernel remote-runtime; run /agent inspect ${agent}${worker ? `; run /machine kernels ${worker}` : ""}; reconnect or relaunch the remote/slice worker before sending prompts to that remote/slice agent`
  }
  if (worker) {
    return `run /kernel remote-runtime; run /machine kernels ${worker}; identify the affected remote/slice agent before sending prompts to it`
  }
  return "run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent"
}

function concreteRecoveryRef(value?: string | null): string {
  const trimmed = value?.trim() ?? ""
  return trimmed && !trimmed.startsWith("<") ? trimmed : ""
}
