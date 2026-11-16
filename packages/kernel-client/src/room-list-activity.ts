import type {
  RoomListItemActivitySummary,
  RoomListSessionActivitySummary,
} from "./kernel-types.js"
import { remoteExtensionAggregateNextAction } from "./home-extension-audit-policy.js"
import { remoteWorkerProviderRunRecoveryAction } from "./provider-run-recovery.js"

export type RoomListActivityBadgeState =
  | "none"
  | "queued"
  | "working"
  | "done"
  | "mixedQueuedDone"
  | "mixedWorkingDone"

export function roomListLifecycleStatusLabel(
  status: string | null | undefined,
  fallback = "-",
): string {
  const normalized = status?.trim().toLowerCase().replace(/[_-]+/g, " ") ?? ""
  const label = normalized
    .split(/\s+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ")
  return label || fallback
}

export function roomListActivityBadgeLabel(state: RoomListActivityBadgeState): string | null {
  switch (state) {
    case "mixedWorkingDone":
      return "Working+Done"
    case "mixedQueuedDone":
      return "Queued+Done"
    case "working":
      return "Working"
    case "queued":
      return "Queued"
    case "done":
      return "Done"
    case "none":
      return null
  }
}

export function roomListSessionStatusLabel(
  session: {
    readonly status?: string | null | undefined
    readonly activity?: Pick<
      RoomListSessionActivitySummary,
      "active_prompt_count" | "queued_prompt_count" | "working_agent_count" | "unread_idle_agent_count" | "pending_interaction_count"
    > | null | undefined
  } | null | undefined,
  fallback = "-",
): string {
  if ((session?.activity?.pending_interaction_count ?? 0) > 0) return "Needs you"
  return roomListActivityBadgeLabel(roomListSessionActivityBadgeState(session?.activity))
    ?? roomListLifecycleStatusLabel(session?.status, fallback)
}

export function roomListTimestampLabel(
  value: number | null | undefined,
  options: {
    readonly missingLabel?: string
    readonly timeZone?: string
    readonly includeTimeZone?: boolean
  } = {},
): string {
  if (!isFiniteNumber(value)) {
    return options.missingLabel ?? "-"
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return options.missingLabel ?? "-"
  }
  const parts = new Intl.DateTimeFormat("en-CA", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZone: options.timeZone,
    timeZoneName: options.includeTimeZone === false ? undefined : "short",
  }).formatToParts(date)
  const values = new Map(parts.map((part) => [part.type, part.value]))
  const label = `${values.get("year")}-${values.get("month")}-${values.get("day")} ${values.get("hour")}:${values.get("minute")}`
  const zone = values.get("timeZoneName")
  return zone ? `${label} ${zone}` : label
}

export function roomListSessionRecencyMs(
  session: {
    readonly last_prompt_sent_at_ms?: number | null | undefined
    readonly last_activity_at_ms?: number | null | undefined
    readonly last_used_at_ms?: number | null | undefined
    readonly created_at_ms?: number | null | undefined
  },
): number {
  return numberOrZero(session.last_activity_at_ms) || numberOrZero(session.created_at_ms)
}

export function roomListSessionActivityHasWork(
  activity: Pick<
    RoomListSessionActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working_agent_count"
  > | null | undefined,
): boolean {
  return (activity?.working_agent_count ?? 0) > 0
    || (activity?.active_prompt_count ?? 0) > 0
    || (activity?.queued_prompt_count ?? 0) > 0
}

export function roomListSessionActivityHasUnreadIdleOutput(
  activity: Pick<RoomListSessionActivitySummary, "unread_idle_agent_count"> | null | undefined,
): boolean {
  return (activity?.unread_idle_agent_count ?? 0) > 0
}

export function roomListSessionActivityBadgeState(
  activity: Pick<
    RoomListSessionActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working_agent_count" | "unread_idle_agent_count" | "pending_interaction_count"
  > | null | undefined,
): RoomListActivityBadgeState {
  const activeWork = roomListSessionActivityHasActiveWork(activity)
  const queued = (activity?.queued_prompt_count ?? 0) > 0
  const done = roomListSessionActivityHasUnreadIdleOutput(activity)
  if (activeWork && done) {
    return "mixedWorkingDone"
  }
  if (activeWork) {
    return "working"
  }
  if (queued && done) {
    return "mixedQueuedDone"
  }
  if (queued) {
    return "queued"
  }
  return done ? "done" : "none"
}

export function roomListSessionActivityWorkLabel(
  activity: Pick<
    RoomListSessionActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working_agent_count"
  > | null | undefined,
  fallback = "-",
): string {
  const workingAgents = activity?.working_agent_count ?? 0
  const activePrompts = activity?.active_prompt_count ?? 0
  const queuedPrompts = activity?.queued_prompt_count ?? 0
  return [
    workingAgents > 0 ? `${workingAgents} working` : "",
    activePrompts > 0 ? `${activePrompts} active prompt${activePrompts === 1 ? "" : "s"}` : "",
    queuedPrompts > 0 ? `${queuedPrompts} queued prompt${queuedPrompts === 1 ? "" : "s"}` : "",
  ].filter(Boolean).join(", ") || fallback
}

export function roomListSessionActivityNextAction(
  session: {
    readonly activity?: Pick<
      RoomListSessionActivitySummary,
      "missing_worker_provider_run_count" | "remote_extension_sync_issue_count" | "remote_extension_pending_revoke_count" | "pending_interaction_count"
    > | null | undefined
  } | null | undefined,
): string | null {
  const activity = session?.activity
  if (!activity) {
    return null
  }
  if ((activity.pending_interaction_count ?? 0) > 0) return "Attach to this session to review the pending request"
  if ((activity.missing_worker_provider_run_count ?? 0) > 0) {
    return remoteWorkerProviderRunRecoveryAction(null, null)
  }
  if ((activity.remote_extension_sync_issue_count ?? 0) > 0) {
    return remoteExtensionAggregateNextAction(activity)
  }
  return null
}

export function roomListSessionRemoteActivityLabel(
  activity: Pick<
    RoomListSessionActivitySummary,
    | "remote_agent_count"
    | "missing_worker_provider_run_count"
    | "home_proxy_agent_count"
    | "remote_extension_sync_issue_count"
    | "remote_extension_pending_revoke_count"
  > | null | undefined,
  fallback = "",
): string {
  const remoteAgents = activity?.remote_agent_count ?? 0
  const workerRunGaps = activity?.missing_worker_provider_run_count ?? 0
  const homeProxyAgents = activity?.home_proxy_agent_count ?? 0
  const remoteExtensionSyncIssues = activity?.remote_extension_sync_issue_count ?? 0
  const pendingRevokes = activity?.remote_extension_pending_revoke_count ?? 0
  return [
    remoteAgents > 0 ? `${remoteAgents} remote/slice ${remoteAgents === 1 ? "agent" : "agents"}` : "",
    workerRunGaps > 0 ? `${workerRunGaps} worker run ${workerRunGaps === 1 ? "gap" : "gaps"}` : "",
    homeProxyAgents > 0 ? `${homeProxyAgents} home-proxy ${homeProxyAgents === 1 ? "agent" : "agents"}` : "",
    remoteExtensionSyncIssues > 0
      ? `${remoteExtensionSyncIssues} extension sync ${remoteExtensionSyncIssues === 1 ? "issue" : "issues"}`
      : "",
    pendingRevokes > 0 ? `${pendingRevokes} pending ${pendingRevokes === 1 ? "revoke" : "revokes"}` : "",
  ].filter(Boolean).join(", ") || fallback
}

export function roomListItemActivityHasWork(
  activity: Pick<
    RoomListItemActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working"
  > | null | undefined,
): boolean {
  return activity?.working === true
    || (activity?.active_prompt_count ?? 0) > 0
    || (activity?.queued_prompt_count ?? 0) > 0
}

export function roomListItemActivityHasUnreadIdleOutput(
  activity: Pick<RoomListItemActivitySummary, "unread_idle_output"> | null | undefined,
): boolean {
  return activity?.unread_idle_output === true
}

export function roomListItemActivityBadgeState(
  activity: Pick<
    RoomListItemActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working" | "unread_idle_output"
  > | null | undefined,
): RoomListActivityBadgeState {
  const activeWork = roomListItemActivityHasActiveWork(activity)
  const queued = (activity?.queued_prompt_count ?? 0) > 0
  const done = roomListItemActivityHasUnreadIdleOutput(activity)
  if (activeWork && done) {
    return "mixedWorkingDone"
  }
  if (activeWork) {
    return "working"
  }
  if (queued && done) {
    return "mixedQueuedDone"
  }
  if (queued) {
    return "queued"
  }
  return done ? "done" : "none"
}

export function roomListItemActivityWorkLabel(
  activity: Pick<
    RoomListItemActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working"
  > | null | undefined,
  fallback = "active work",
): string {
  const activePrompts = activity?.active_prompt_count ?? 0
  const queuedPrompts = activity?.queued_prompt_count ?? 0
  return [
    activity?.working === true ? "working" : "",
    activePrompts > 0 ? `${activePrompts} active prompt${activePrompts === 1 ? "" : "s"}` : "",
    queuedPrompts > 0 ? `${queuedPrompts} queued prompt${queuedPrompts === 1 ? "" : "s"}` : "",
  ].filter(Boolean).join(", ") || fallback
}

function numberOrZero(value: number | null | undefined): number {
  return isFiniteNumber(value) ? value : 0
}

function roomListSessionActivityHasActiveWork(
  activity: Pick<
    RoomListSessionActivitySummary,
    "active_prompt_count" | "working_agent_count"
  > | null | undefined,
): boolean {
  return (activity?.working_agent_count ?? 0) > 0
    || (activity?.active_prompt_count ?? 0) > 0
}

function roomListItemActivityHasActiveWork(
  activity: Pick<
    RoomListItemActivitySummary,
    "active_prompt_count" | "working"
  > | null | undefined,
): boolean {
  return activity?.working === true
    || (activity?.active_prompt_count ?? 0) > 0
}

function isFiniteNumber(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value)
}
