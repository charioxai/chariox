import type { AgentRuntimeActivityProjection } from "./agent-activity.js"
import type { PromptQueueItem, RuntimeSession } from "./kernel-types.js"
import {
  externalProviderObservedExplicitIdentityFields,
} from "./external-provider-observation.js"
import { promptOriginFromRecord } from "./prompt-origin.js"
import {
  sessionAgentActivityRecordForAgent,
  sessionHasAgentActivityProjection,
  sessionHasPromptStateProjection,
  sessionPromptStateRecordForAgent,
  sessionProjectedPromptActivityForAgent,
} from "./session-agent-prompt-state.js"

export const QUEUED_PROMPT_STALE_REASON = "This prompt is no longer waiting in the queue."
export const QUEUED_PROMPT_CONTROLS_UNAVAILABLE_REASON =
  "Queued prompt controls are unavailable in the current kernel snapshot."

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

export type QueuedPromptAction = "steer" | "cancel"

export type QueuedPromptActionState = {
  readonly action: QueuedPromptAction
  readonly enabled: boolean
  readonly disabled: boolean
  readonly disabledReason: string | null
}

export type ProjectedQueuedPrompt = QueuedPromptActionability & {
  readonly id: string
  readonly pendingPromptId: string | null
  readonly sourceAttachmentId: string
  readonly targetAgentId: string | null
  readonly prompt: string
  readonly promptOrigin: string | null
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
  readonly createdAtMs?: number | null
  readonly attachmentCount: number
}

export type QueuedPromptProjection =
  | { readonly action: "ignore" }
  | { readonly action: "preserve" }
  | { readonly action: "replace"; readonly prompts: readonly ProjectedQueuedPrompt[] }

export function queuedPromptActionability(
  promptStatus: string | null | undefined,
  control: QueuedPromptControlInput | null | undefined = null,
): QueuedPromptActionability {
  const status = normalizeQueuedPromptStatus(control?.status ?? promptStatus)
  const queued = queuedPromptStatusIsQueued(status)
  const controlsProjected = control !== null && control !== undefined
  const canSteer = controlsProjected ? control.can_steer === true : false
  const canCancel = controlsProjected ? control.can_cancel === true : false
  const missingProjectedReason = queued ? null : QUEUED_PROMPT_STALE_REASON
  const missingControlReason = queued
    ? QUEUED_PROMPT_CONTROLS_UNAVAILABLE_REASON
    : QUEUED_PROMPT_STALE_REASON
  const steerDisabledReason = hasOwn(control, "steer_disabled_reason")
    ? control.steer_disabled_reason ?? null
    : controlsProjected ? missingProjectedReason : missingControlReason
  const cancelDisabledReason = hasOwn(control, "cancel_disabled_reason")
    ? control.cancel_disabled_reason ?? null
    : controlsProjected ? missingProjectedReason : missingControlReason
  return {
    status,
    steerDisabled: !canSteer,
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

export function queuedPromptActionState(
  item: Pick<QueuedPromptActionability, "canSteer" | "canCancel" | "steerDisabledReason" | "cancelDisabledReason">,
  action: QueuedPromptAction,
): QueuedPromptActionState {
  const enabled = action === "steer" ? item.canSteer === true : item.canCancel === true
  return {
    action,
    enabled,
    disabled: !enabled,
    disabledReason: action === "steer" ? item.steerDisabledReason : item.cancelDisabledReason,
  }
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
    && current.promptOrigin === next.promptOrigin
    && current.externalProvider === next.externalProvider
    && current.externalProviderSessionId === next.externalProviderSessionId
    && current.externalProviderTurnId === next.externalProviderTurnId
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
  return queuedPromptControlForPromptIds(controls, [promptId])
}

export function queuedPromptControlForPromptIds(
  controls: Record<string, QueuedPromptControlInput | null | undefined> | null | undefined,
  promptIds: readonly (string | null | undefined)[],
): QueuedPromptControlInput | null {
  const lookupPromptIds = uniqueNonBlankStrings(promptIds)
  if (!controls || lookupPromptIds.length === 0) {
    return null
  }
  for (const lookupPromptId of lookupPromptIds) {
    const control = controls[lookupPromptId] ?? null
    if (!control) {
      continue
    }
    const projectedPromptId = nonBlankString(control.prompt_id)
    if (projectedPromptId && !lookupPromptIds.includes(projectedPromptId)) {
      continue
    }
    return control
  }
  return null
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
  return []
}

export function queuedPromptProjectionForAgent(
  session: RuntimeSession,
  agentId: string,
): QueuedPromptProjection {
  if (!sessionHasAgentActivityProjection(session) && !sessionHasPromptStateProjection(session)) {
    return { action: "ignore" }
  }
  const prompts = queuedPromptsForAgent(session, agentId)
  if (prompts === null) {
    return { action: "preserve" }
  }
  const controls = sessionAgentActivityRecordForAgent(session, agentId)?.queued_prompt_controls
  return {
    action: "replace",
    prompts: sortProjectedQueuedPrompts(prompts.flatMap((prompt): ProjectedQueuedPrompt[] => {
      const projected = projectQueuedPrompt(prompt, {
        fallbackTargetAgentId: agentId,
        control: queuedPromptControlForPromptIds(controls, [
          prompt.pending_prompt_id,
          prompt.id,
        ]),
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
  const promptId = nonBlankString(prompt.id)
  if (!promptId || !prompt.prompt) {
    return null
  }
  const pendingPromptId = nonBlankString(prompt.pending_prompt_id)
  const createdAtMs = finiteNumber(prompt.created_at_ms)
  const externalIdentity = externalProviderObservedExplicitIdentityFields(prompt)
  return {
    id: pendingPromptId ?? promptId,
    pendingPromptId,
    sourceAttachmentId: nonBlankString(prompt.source_attachment_id) ?? "",
    targetAgentId: nonBlankString(prompt.target_agent_id) ?? nonBlankString(options.fallbackTargetAgentId) ?? null,
    prompt: prompt.prompt,
    promptOrigin: promptOriginFromRecord(prompt),
    ...externalIdentity,
    ...(createdAtMs !== null ? { createdAtMs } : {}),
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

export function queuedPromptActionLabel(action: QueuedPromptAction, focusedPrimary: boolean): string {
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

function uniqueNonBlankStrings(values: readonly (string | null | undefined)[]): string[] {
  return [...new Set(values.flatMap((value) => {
    const normalized = nonBlankString(value)
    return normalized ? [normalized] : []
  }))]
}

function finiteNumber(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function compareProjectedQueuedPromptOrder(
  left: ProjectedQueuedPrompt,
  right: ProjectedQueuedPrompt,
): number {
  const leftCreated = queuedPromptCreatedAtMs(left)
  const rightCreated = queuedPromptCreatedAtMs(right)
  return leftCreated - rightCreated || left.id.localeCompare(right.id)
}

function queuedPromptCreatedAtMs(prompt: Pick<ProjectedQueuedPrompt, "createdAtMs">): number {
  return typeof prompt.createdAtMs === "number" && Number.isFinite(prompt.createdAtMs)
    ? prompt.createdAtMs
    : Number.MAX_SAFE_INTEGER
}

function projectedActivityAllowsPromptQueue(projection: AgentRuntimeActivityProjection): boolean {
  if (projection.queuedPromptCountExplicit) {
    return projection.queuedPromptCount > 0
  }
  return projection.busy
}
