import type {
  WaitingRoomPublicItemActivitySummary,
  WaitingRoomSessionActivitySummary,
} from "./kernel-types.js"

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
