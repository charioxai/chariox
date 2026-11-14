import type { AgentTrigger, AgentTriggerBusyPolicy, RuntimeSession, TriggerDelivery } from "./kernel-types-session.js"

/** One active-trigger row for an agent pane stripe (TUI footer, web pane). */
export type AgentTriggerStripItem = {
  readonly id: string
  readonly label: string
  readonly busyPolicy: AgentTriggerBusyPolicy
  readonly enabled: boolean
  readonly nextRunAtMs: number | null
  /** The schedule fired its max runs. */
  readonly done: boolean
  readonly fired: number
  readonly lastError: string | null
  /** A notification trigger whose source's kernel is down: shown as "offline", like the workflow picker. */
  readonly offline: boolean
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
    nextRunAtMs: trigger.enabled && trigger.source.kind === "schedule" && !agentTriggerDone(trigger) ? trigger.source.next_run_at_ms : null,
    done: agentTriggerDone(trigger),
    ...(localUrl ? { localUrl } : {}),
    fired: trigger.fired,
    lastError: trigger.last_error?.trim() || null,
    offline: trigger.enabled && trigger.source.kind === "notification" && !trigger.source.source_available,
  }
}

export function agentTriggerSourceLabel(trigger: Pick<AgentTrigger, "source">): string {
  if (trigger.source.kind === "http") return `HTTP ${trigger.source.methods.join(" and ")} ${trigger.source.route}`
  if (trigger.source.kind === "notification") return `Notification · ${trigger.source.display_name}`
  const { trigger: schedule, repeat } = trigger.source
  if (schedule.kind === "cron") return `${schedule.expression} · ${schedule.timezone}`
  return `${repeat === "every" ? "every" : "in"} ${formatInterval(schedule.every_seconds)}`
}

/** A repeating schedule that fired its max runs: it stays listed, and never fires again. */
export function agentTriggerDone(trigger: Pick<AgentTrigger, "source" | "fired">): boolean {
  return trigger.source.kind === "schedule" && typeof trigger.source.max_runs === "number" && trigger.fired >= trigger.source.max_runs
}

function formatInterval(seconds: number): string {
  if (seconds % 3600 === 0) return `${seconds / 3600}h`
  if (seconds % 60 === 0) return `${seconds / 60}m`
  return `${seconds}s`
}

export function agentTriggerLocalUrl(trigger: Pick<AgentTrigger, "source">): string | null {
  return trigger.source.kind === "http" ? `http://127.0.0.1:${trigger.source.port}/${encodeURIComponent(trigger.source.ingress_id)}/` : null
}

/** Unconfirmed firings outlive their one-shot trigger and remain actionable. */
export function unconfirmedTriggerDeliveriesForAgent(
  session: Pick<RuntimeSession, "trigger_deliveries"> | null | undefined,
  agentId: string | null | undefined,
): TriggerDelivery[] {
  if (!agentId) return []
  return (session?.trigger_deliveries ?? []).filter(delivery => delivery.agent_id === agentId && delivery.state === "not_confirmed")
}

export function triggerPromptOriginLabel(origin: string | null | undefined): string | null {
  switch (origin) {
    case "schedule": return "Schedule"
    case "http": return "HTTP"
    case "notification": return "Notification"
    case "app_event": return "App event"
    default: return null
  }
}
