import type { AgentRuntimeActivityProjection } from "./agent-activity.js"
import type { PromptQueueItem, RuntimeSession } from "./kernel-types.js"
import { promptOriginIsExternal } from "./prompt-origin.js"
import {
  sessionAgentActivityRecordForAgent,
  sessionPromptStateRecordForAgent,
  sessionProjectedPromptActivityForAgent,
  type SessionProjectedPromptActivity,
} from "./session-agent-prompt-state.js"

export const QUEUED_PROMPT_STALE_REASON = "This prompt is no longer waiting in the queue."
export const QUEUED_PROMPT_STEER_EXTERNAL_REASON =
  "Steering is unavailable while the active provider turn was started outside Arroba."

export type QueuedPromptControlInput = {
  readonly prompt_id?: string | null
  readonly status?: string | null
  readonly can_steer?: boolean | null
  readonly can_cancel?: boolean | null
  readonly steer_disabled_reason?: string | null
  readonly cancel_disabled_reason?: string | null
}

export type QueuedPromptActionability = {
  readonly status: string
  readonly steerDisabled: boolean
  readonly canSteer: boolean
  readonly canCancel: boolean
  readonly steerDisabledReason: string | null
  readonly cancelDisabledReason: string | null
}

export type ProjectedQueuedPrompt = QueuedPromptActionability & {
  readonly id: string
  readonly pendingPromptId: string | null
  readonly sourceAttachmentId: string
  readonly targetAgentId: string | null
  readonly prompt: string
  readonly createdAtMs?: number | null
  readonly attachmentCount: number
}

export type QueuedPromptProjection =
  | { readonly action: "preserve" }
  | { readonly action: "replace"; readonly prompts: readonly ProjectedQueuedPrompt[] }

export function queuedPromptActionability(
  promptStatus: string | null | undefined,
  control: QueuedPromptControlInput | null | undefined = null,
): QueuedPromptActionability {
  const status = normalizeQueuedPromptStatus(control?.status ?? promptStatus)
  const queued = queuedPromptStatusIsQueued(status)
  const canSteer = control?.can_steer ?? queued
  const canCancel = control?.can_cancel ?? queued
  const steerDisabledReason = hasOwn(control, "steer_disabled_reason")
    ? control.steer_disabled_reason ?? null
    : queuedPromptSteerDisabledReason(status)
  const cancelDisabledReason = hasOwn(control, "cancel_disabled_reason")
    ? control.cancel_disabled_reason ?? null
    : queuedPromptCancelDisabledReason(status)
  return {
    status,
    steerDisabled: !canSteer && Boolean(steerDisabledReason),
    canSteer,
    canCancel,
    steerDisabledReason,
    cancelDisabledReason,
  }
}

export function queuedPromptActionabilityMatches(
  current: QueuedPromptActionability,
  next: QueuedPromptActionability,
): boolean {
  return current.status === next.status
    && current.steerDisabled === next.steerDisabled
    && current.canSteer === next.canSteer
    && current.canCancel === next.canCancel
    && current.steerDisabledReason === next.steerDisabledReason
    && current.cancelDisabledReason === next.cancelDisabledReason
}

export function projectedQueuedPromptMatches(
  current: ProjectedQueuedPrompt,
  next: ProjectedQueuedPrompt,
): boolean {
  return current.id === next.id
    && current.pendingPromptId === next.pendingPromptId
    && current.sourceAttachmentId === next.sourceAttachmentId
    && current.targetAgentId === next.targetAgentId
    && current.prompt === next.prompt
    && (current.createdAtMs ?? null) === (next.createdAtMs ?? null)
    && current.attachmentCount === next.attachmentCount
    && queuedPromptActionabilityMatches(current, next)
}

export function projectedQueuedPromptListsMatch(
  current: readonly ProjectedQueuedPrompt[],
  next: readonly ProjectedQueuedPrompt[],
): boolean {
  return current.length === next.length
    && current.every((prompt, index) => projectedQueuedPromptMatches(prompt, next[index]!))
}

export function queuedPromptControlForPrompt(
  controls: Record<string, QueuedPromptControlInput | null | undefined> | null | undefined,
  promptId: string | null | undefined,
): QueuedPromptControlInput | null {
  if (!controls || !promptId) {
    return null
  }
  const control = controls[promptId] ?? null
  if (!control) {
    return null
  }
  const projectedPromptId = nonBlankString(control.prompt_id)
  if (projectedPromptId && projectedPromptId !== promptId) {
    return null
  }
  return control
}

export function queuedPromptsForAgent(
  session: RuntimeSession,
  agentId: string,
): readonly PromptQueueItem[] | null {
  const projectedActivity = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projectedActivity === "not_found" || projectedActivity === "idle") {
    return []
  }
  if (projectedActivity && !projectedActivityAllowsPromptQueue(projectedActivity)) {
    return []
  }
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.queued_prompts ?? []
  }
  if (projectedActivity) {
    return null
  }
  return session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId)
}

export function queuedPromptProjectionForAgent(
  session: RuntimeSession,
  agentId: string,
): QueuedPromptProjection {
  const prompts = queuedPromptsForAgent(session, agentId)
  if (prompts === null) {
    return { action: "preserve" }
  }
  const projectedActivity = sessionProjectedPromptActivityForAgent(session, agentId)
  const disableSteeringBehindExternalTurn = projectedActivityHasExternalActiveTurn(projectedActivity)
  const controls = sessionAgentActivityRecordForAgent(session, agentId)?.queued_prompt_controls
  return {
    action: "replace",
    prompts: sortProjectedQueuedPrompts(prompts.flatMap((prompt): ProjectedQueuedPrompt[] => {
      const promptId = prompt.pending_prompt_id ?? prompt.id
      const projected = projectQueuedPrompt(prompt, {
        fallbackTargetAgentId: agentId,
        control: queuedPromptControlWithActivityFallback(
          controls,
          promptId,
          disableSteeringBehindExternalTurn,
        ),
      })
      return projected ? [projected] : []
    })),
  }
}

export function projectQueuedPrompt(
  prompt: PromptQueueItem,
  options: {
    readonly fallbackTargetAgentId?: string | null
    readonly control?: QueuedPromptControlInput | null
  } = {},
): ProjectedQueuedPrompt | null {
  if (!prompt.id || !prompt.prompt) {
    return null
  }
  const pendingPromptId = prompt.pending_prompt_id ?? null
  return {
    id: pendingPromptId ?? prompt.id,
    pendingPromptId,
    sourceAttachmentId: prompt.source_attachment_id ?? "",
    targetAgentId: prompt.target_agent_id ?? options.fallbackTargetAgentId ?? null,
    prompt: prompt.prompt,
    ...(prompt.created_at_ms !== undefined ? { createdAtMs: prompt.created_at_ms } : {}),
    attachmentCount: Array.isArray(prompt.attachments) ? prompt.attachments.length : 0,
    ...queuedPromptActionability(prompt.status, options.control),
  }
}

export function sortProjectedQueuedPrompts(
  prompts: readonly ProjectedQueuedPrompt[],
): readonly ProjectedQueuedPrompt[] {
  return [...prompts].sort(compareProjectedQueuedPromptOrder)
}

export function normalizeQueuedPromptStatus(status: string | null | undefined): string {
  return status?.trim().toLowerCase() || "queued"
}

export function queuedPromptStatusLabel(status: string | null | undefined): string {
  return normalizeQueuedPromptStatus(status).replace(/[_-]+/g, " ")
}

export function queuedPromptAttachmentLabel(attachmentCount: number | null | undefined): string {
  const count = Math.max(0, Math.trunc(attachmentCount ?? 0))
  if (count <= 0) {
    return ""
  }
  return ` · ${count} file${count === 1 ? "" : "s"}`
}

export function queuedPromptMetaLabel(
  item: {
    readonly status?: string | null | undefined
    readonly attachmentCount?: number | null | undefined
  },
): string {
  return `${queuedPromptStatusLabel(item.status)}${queuedPromptAttachmentLabel(item.attachmentCount)}`
}

export function queuedPromptTitleLabel(count: number, focused: boolean): string {
  const normalizedCount = Math.max(0, Math.trunc(count))
  const countLabel = `QUEUE • ${normalizedCount} prompt${normalizedCount === 1 ? "" : "s"}`
  return focused
    ? `${countLabel} • J/K select • S steer • C cancel`
    : countLabel
}

export function queuedPromptActionLabel(action: "steer" | "cancel", focusedPrimary: boolean): string {
  if (!focusedPrimary) {
    return action
  }
  return action === "steer" ? "S" : "C"
}

export function queuedPromptStatusIsQueued(status: string): boolean {
  return normalizeQueuedPromptStatus(status) === "queued"
}

export function queuedPromptSteerDisabledReason(status: string): string | null {
  return queuedPromptStatusIsQueued(status) ? null : QUEUED_PROMPT_STALE_REASON
}

export function queuedPromptCancelDisabledReason(status: string): string | null {
  return queuedPromptStatusIsQueued(status) ? null : QUEUED_PROMPT_STALE_REASON
}

function hasOwn<T extends object, K extends PropertyKey>(
  value: T | null | undefined,
  key: K,
): value is T & Record<K, unknown> {
  return Boolean(value && Object.prototype.hasOwnProperty.call(value, key))
}

function nonBlankString(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
}

function compareProjectedQueuedPromptOrder(
  left: ProjectedQueuedPrompt,
  right: ProjectedQueuedPrompt,
): number {
  const leftCreated = left.createdAtMs ?? Number.MAX_SAFE_INTEGER
  const rightCreated = right.createdAtMs ?? Number.MAX_SAFE_INTEGER
  return leftCreated - rightCreated || left.id.localeCompare(right.id)
}

function projectedActivityAllowsPromptQueue(projection: AgentRuntimeActivityProjection): boolean {
  if (projection.queuedPromptCountExplicit && projection.queuedPromptCount === 0) {
    return false
  }
  return projection.busy
}

function queuedPromptControlWithActivityFallback(
  controls: Record<string, QueuedPromptControlInput | null | undefined> | null | undefined,
  promptId: string | null | undefined,
  disableSteeringBehindExternalTurn: boolean,
): QueuedPromptControlInput | null {
  const control = queuedPromptControlForPrompt(controls, promptId)
  if (control || !disableSteeringBehindExternalTurn || !promptId) {
    return control
  }
  return {
    prompt_id: promptId,
    can_steer: false,
    steer_disabled_reason: QUEUED_PROMPT_STEER_EXTERNAL_REASON,
  }
}

function projectedActivityHasExternalActiveTurn(activity: SessionProjectedPromptActivity): boolean {
  if (!activity || activity === "idle" || activity === "not_found") {
    return false
  }
  const projection = activity
  return promptOriginIsExternal(projection.activeTurnPromptOrigin)
    || Boolean(
      projection.activeTurnExternalProvider
        || projection.activeTurnExternalProviderSessionId
        || projection.activeTurnExternalProviderTurnId,
    )
}
