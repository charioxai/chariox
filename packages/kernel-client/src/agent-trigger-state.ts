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
  readonly localUrl?: string
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
  const localUrl = agentTriggerLocalUrl(trigger)
  return {
    id: trigger.id,
    label: agentTriggerSourceLabel(trigger),
    busyPolicy: trigger.busy_policy,
    enabled: trigger.enabled,
    nextRunAtMs: trigger.enabled && trigger.source.kind === "schedule" ? trigger.source.next_run_at_ms : null,
    ...(localUrl ? { localUrl } : {}),
    fired: trigger.fired,
    lastError: trigger.last_error?.trim() || null,
  }
}

export function agentTriggerSourceLabel(trigger: Pick<AgentTrigger, "source">): string {
  if (trigger.source.kind === "http") return "HTTP"
  const { interval_seconds: seconds, repeat } = trigger.source
  return `${repeat === "every" ? "every" : "in"} ${formatInterval(seconds)}`
}

function formatInterval(seconds: number): string {
  if (seconds % 3600 === 0) return `${seconds / 3600}h`
  if (seconds % 60 === 0) return `${seconds / 60}m`
  return `${seconds}s`
}

export function agentTriggerLocalUrl(trigger: Pick<AgentTrigger, "source">): string | null {
  return trigger.source.kind === "http" ? `http://127.0.0.1:${trigger.source.port}/${encodeURIComponent(trigger.source.ingress_id)}/` : null
}
