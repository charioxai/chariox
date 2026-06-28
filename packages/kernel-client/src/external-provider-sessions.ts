export type ExternalProviderSessionCapabilities = {
  readonly can_read_history?: boolean
}

export type ExternalProviderSessionRecord = {
  readonly external_session_id: string
  readonly provider: string
  readonly provider_session_id: string
  readonly title?: string | null
  readonly title_source?: string | null
  readonly first_prompt_preview?: string | null
  readonly created_at_ms?: number | null
  readonly last_modified_at_ms: number
  readonly worktree_path?: string | null
  readonly account_profile?: string | null
  readonly capabilities?: ExternalProviderSessionCapabilities
}

export type ExternalProviderSessionSelection = {
  readonly selectedExternalProviderSessionId?: string | null
  readonly selectedExternalProviderSessionIndex?: number | null
}

export function externalProviderSessionsSorted<T extends ExternalProviderSessionRecord>(
  sessions: readonly T[] | null | undefined,
): T[] {
  return [...(sessions ?? [])].sort(compareExternalProviderSessions)
}

export function mergeExternalProviderSessions<T extends ExternalProviderSessionRecord>(
  ...groups: readonly (readonly T[])[]
): T[] {
  const sessionsById = new Map<string, T>()
  for (const group of groups) {
    for (const session of group) {
      const existing = sessionsById.get(session.external_session_id)
      if (!existing || externalProviderSessionModifiedMs(session) > externalProviderSessionModifiedMs(existing)) {
        sessionsById.set(session.external_session_id, session)
      }
    }
  }
  return Array.from(sessionsById.values())
}

export function mergeExternalProviderSessionsSorted<T extends ExternalProviderSessionRecord>(
  ...groups: readonly (readonly T[])[]
): T[] {
  return externalProviderSessionsSorted(mergeExternalProviderSessions(...groups))
}

export function externalProviderSessionSelectionIndex(
  sessions: readonly ExternalProviderSessionRecord[],
  selection: ExternalProviderSessionSelection,
): number {
  const selectedId = selection.selectedExternalProviderSessionId?.trim()
  if (selectedId) {
    const index = sessions.findIndex((session) => session.external_session_id === selectedId)
    if (index >= 0) {
      return index
    }
  }
  return clampExternalProviderSessionIndex(selection.selectedExternalProviderSessionIndex ?? 0, sessions.length)
}

export function externalProviderSessionAtSelection<T extends ExternalProviderSessionRecord>(
  sessions: readonly T[],
  selection: ExternalProviderSessionSelection,
): T | null {
  return sessions[externalProviderSessionSelectionIndex(sessions, selection)] ?? null
}

export function externalProviderSessionTitle(session: ExternalProviderSessionRecord): string {
  return session.title?.trim()
    || session.first_prompt_preview?.trim()
    || session.provider_session_id
    || session.external_session_id
}

export function externalProviderSessionModeLabel(_session: ExternalProviderSessionRecord): string {
  return "observed"
}

export function externalProviderSessionModifiedLabel(
  session: ExternalProviderSessionRecord,
  options: {
    readonly utcSuffix?: boolean
  } = {},
): string {
  const value = externalProviderSessionModifiedMs(session)
  if (!value) {
    return "-"
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return "-"
  }
  const label = date.toISOString().replace("T", " ").slice(0, 16)
  return options.utcSuffix ? `${label} UTC` : label
}

export function externalProviderSessionModifiedMs(session: ExternalProviderSessionRecord): number {
  return typeof session.last_modified_at_ms === "number" ? session.last_modified_at_ms : 0
}

function compareExternalProviderSessions(
  left: ExternalProviderSessionRecord,
  right: ExternalProviderSessionRecord,
): number {
  const modified = externalProviderSessionModifiedMs(right) - externalProviderSessionModifiedMs(left)
  if (modified !== 0) {
    return modified
  }
  const provider = left.provider.localeCompare(right.provider)
  if (provider !== 0) {
    return provider
  }
  return left.provider_session_id.localeCompare(right.provider_session_id)
}

function clampExternalProviderSessionIndex(index: number, length: number): number {
  if (length <= 0) {
    return 0
  }
  const finiteIndex = Number.isFinite(index) ? Math.floor(index) : 0
  return Math.max(0, Math.min(finiteIndex, length - 1))
}
