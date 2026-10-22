import type { AgentTrigger, AgentTriggerBusyPolicy, RuntimeSession } from "./kernel-types-session.js"

/** One active-trigger row for an agent pane stripe (TUI footer, web pane). */
export type AgentTriggerStripItem = {
  readonly id: string
  readonly label: string
  readonly busyPolicy: AgentTriggerBusyPolicy
  readonly enabled: boolean
  readonly nextRunAtMs: number | null
  readonly fired: number
  readonly lastError: string | null
}

export function agentTriggersForAgent(
  session: Pick<RuntimeSession, "agent_triggers"> | null | undefined,
  agentId: string | null | undefined,
): AgentTriggerStripItem[] {
  if (!agentId) return []
  return (session?.agent_triggers ?? [])
    .filter((trigger) => trigger.agent_id === agentId)
    .map(agentTriggerStripItem)
    .sort((left, right) => (left.nextRunAtMs ?? Infinity) - (right.nextRunAtMs ?? Infinity) || left.id.localeCompare(right.id))
}

function agentTriggerStripItem(trigger: AgentTrigger): AgentTriggerStripItem {
  return {
    id: trigger.id,
    label: agentTriggerSourceLabel(trigger),
    busyPolicy: trigger.busy_policy,
    enabled: trigger.enabled,
    nextRunAtMs: trigger.enabled ? trigger.source.next_run_at_ms : null,
    fired: trigger.fired,
    lastError: trigger.last_error?.trim() || null,
  }
}

export function agentTriggerSourceLabel(trigger: Pick<AgentTrigger, "source">): string {
  const { interval_seconds: seconds, repeat } = trigger.source
  return `${repeat === "every" ? "every" : "in"} ${formatInterval(seconds)}`
}

function formatInterval(seconds: number): string {
  if (seconds % 3600 === 0) return `${seconds / 3600}h`
  if (seconds % 60 === 0) return `${seconds / 60}m`
  return `${seconds}s`
}
