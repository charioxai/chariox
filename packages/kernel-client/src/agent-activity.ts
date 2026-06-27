export type AgentRuntimeActivityBusyInput = {
  readonly busy?: boolean | null
  readonly status?: string | null
  readonly prompt_status?: string | null
  readonly active_turn?: unknown | null
}

export function agentRuntimeActivityIsBusy(
  activity: AgentRuntimeActivityBusyInput | null | undefined,
): boolean {
  const status = normalizeAgentRuntimeActivityStatus(activity?.status)
  const promptStatus = normalizeAgentRuntimePromptStatus(activity?.prompt_status)
  return Boolean(activity && (
    activity.busy === true
    || status === "working"
    || agentRuntimePromptStatusIsActive(promptStatus)
    || agentRuntimeActiveTurnIsBusy(activity.active_turn)
  ))
}

export function normalizeAgentRuntimeActivityStatus(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase()
  return normalized || null
}

export function normalizeAgentRuntimePromptStatus(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase()
  return normalized || null
}

export function agentRuntimePromptStatusIsActive(value: string | null): boolean {
  return value === "queued"
    || value === "running"
    || value === "cancelling"
    || value === "settling"
}

export function agentRuntimePromptStatusIsActivePrompt(value: string | null): boolean {
  return value === "running"
    || value === "cancelling"
    || value === "settling"
}

function agentRuntimeActiveTurnIsBusy(activeTurn: unknown): boolean {
  if (!activeTurn || typeof activeTurn !== "object" || Array.isArray(activeTurn)) {
    return false
  }
  const rawStatus = (activeTurn as { readonly status?: unknown }).status
  const status = normalizeAgentRuntimePromptStatus(typeof rawStatus === "string" ? rawStatus : null)
  return status === null || agentRuntimePromptStatusIsActive(status)
}
