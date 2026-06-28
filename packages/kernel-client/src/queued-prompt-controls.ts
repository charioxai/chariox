export const QUEUED_PROMPT_STALE_REASON = "This prompt is no longer waiting in the queue."

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

export function normalizeQueuedPromptStatus(status: string | null | undefined): string {
  return status?.trim().toLowerCase() || "queued"
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
