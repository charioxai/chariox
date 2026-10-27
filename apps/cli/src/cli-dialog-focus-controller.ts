export type CliDialogFocusTarget = {
  readonly id?: string | number
  readonly isDestroyed: boolean
  readonly focused?: boolean
  focus(): void
  blur(): void
}

export type CliDialogFocusSnapshot = {
  id: string
  type: string | null
  destroyed: boolean
  focused: boolean
}

export function describeCliDialogFocusTarget(
  target: CliDialogFocusTarget | null | undefined,
): CliDialogFocusSnapshot | null {
  if (!target) {
    return null
  }
  return {
    id: String(target.id ?? ""),
    type: target.constructor?.name ?? null,
    destroyed: target.isDestroyed,
    focused: Boolean(target.focused),
  }
}

export function resolveCliDialogFocusTarget<T extends CliDialogFocusTarget>(
  current: T | null | undefined,
  fallback: T | null | undefined,
): T | null {
  if (current && !current.isDestroyed) {
    return current
  }
  if (fallback && !fallback.isDestroyed) {
    return fallback
  }
  return null
}

export function captureCliDialogFocus<T extends CliDialogFocusTarget>(
  current: T | null | undefined,
  fallback: T | null | undefined,
): T | null {
  const target = resolveCliDialogFocusTarget(current, fallback)
  target?.blur()
  return target
}

export function restoreCliDialogFocus(
  target: CliDialogFocusTarget | null | undefined,
): boolean {
  if (!target || target.isDestroyed) {
    return false
  }
  target.focus()
  return true
}
