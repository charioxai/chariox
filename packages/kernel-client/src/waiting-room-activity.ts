import type {
  WaitingRoomPublicItemActivitySummary,
  WaitingRoomSessionActivitySummary,
} from "./kernel-types.js"

export type WaitingRoomActivityBadgeState = "none" | "working" | "done" | "mixedWorkingDone"

export function waitingRoomSessionActivityHasWork(
  activity: Pick<
    WaitingRoomSessionActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working_agent_count"
  > | null | undefined,
): boolean {
  return (activity?.working_agent_count ?? 0) > 0
    || (activity?.active_prompt_count ?? 0) > 0
    || (activity?.queued_prompt_count ?? 0) > 0
}

export function waitingRoomSessionActivityHasUnreadIdleOutput(
  activity: Pick<WaitingRoomSessionActivitySummary, "unread_idle_agent_count"> | null | undefined,
): boolean {
  return (activity?.unread_idle_agent_count ?? 0) > 0
}

export function waitingRoomSessionActivityBadgeState(
  activity: Pick<
    WaitingRoomSessionActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working_agent_count" | "unread_idle_agent_count"
  > | null | undefined,
): WaitingRoomActivityBadgeState {
  const working = waitingRoomSessionActivityHasWork(activity)
  const done = waitingRoomSessionActivityHasUnreadIdleOutput(activity)
  if (working && done) {
    return "mixedWorkingDone"
  }
  if (working) {
    return "working"
  }
  return done ? "done" : "none"
}

export function waitingRoomSessionActivityWorkLabel(
  activity: Pick<
    WaitingRoomSessionActivitySummary,
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

export function waitingRoomItemActivityHasWork(
  activity: Pick<
    WaitingRoomPublicItemActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working"
  > | null | undefined,
): boolean {
  return activity?.working === true
    || (activity?.active_prompt_count ?? 0) > 0
    || (activity?.queued_prompt_count ?? 0) > 0
}

export function waitingRoomItemActivityHasUnreadIdleOutput(
  activity: Pick<WaitingRoomPublicItemActivitySummary, "unread_idle_output"> | null | undefined,
): boolean {
  return activity?.unread_idle_output === true
}

export function waitingRoomItemActivityBadgeState(
  activity: Pick<
    WaitingRoomPublicItemActivitySummary,
    "active_prompt_count" | "queued_prompt_count" | "working" | "unread_idle_output"
  > | null | undefined,
): WaitingRoomActivityBadgeState {
  if (waitingRoomItemActivityHasWork(activity)) {
    return "working"
  }
  return waitingRoomItemActivityHasUnreadIdleOutput(activity) ? "done" : "none"
}

export function waitingRoomItemActivityWorkLabel(
  activity: Pick<
    WaitingRoomPublicItemActivitySummary,
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
