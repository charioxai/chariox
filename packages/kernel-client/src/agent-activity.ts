export type AgentRuntimeActivityBusyInput = {
  readonly busy?: boolean | null
  readonly status?: string | null
  readonly prompt_status?: string | null
  readonly active_turn?: unknown | null
}

export function agentRuntimeActivityIsBusy(
  activity: AgentRuntimeActivityBusyInput | null | undefined,
): boolean {
  return Boolean(activity && (
    activity.busy === true
    || activity.status === "working"
    || (activity.prompt_status !== undefined
      && activity.prompt_status !== null
      && activity.prompt_status !== "none")
    || activity.active_turn
  ))
}
