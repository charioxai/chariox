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

export type ExternalProviderSessionPageLike<T extends ExternalProviderSessionRecord = ExternalProviderSessionRecord> = {
  readonly sessions?: readonly T[] | null
  readonly externalProviderSessions?: readonly T[] | null
  readonly hasMore?: boolean | null
  readonly has_more?: boolean | null
  readonly externalProviderSessionsHasMore?: boolean | null
  readonly nextCursor?: string | null
  readonly next_cursor?: string | null
  readonly externalProviderSessionsNextCursor?: string | null
}

export type ExternalProviderSessionPageState = {
  readonly hasMore: boolean
  readonly nextCursor: string | null
}

export type ExternalProviderSessionPage<T extends ExternalProviderSessionRecord = ExternalProviderSessionRecord> =
  ExternalProviderSessionPageState & {
    readonly sessions: T[]
  }

export function externalProviderSessionsSorted<T extends ExternalProviderSessionRecord>(
  sessions: readonly T[] | null | undefined,
): T[] {
  return [...(sessions ?? [])].sort(compareExternalProviderSessions)
}

export function externalProviderSessionPage<T extends ExternalProviderSessionRecord>(
  page: ExternalProviderSessionPageLike<T> | null | undefined,
): ExternalProviderSessionPage<T> {
  return {
    sessions: externalProviderSessionPageSessions(page),
    ...externalProviderSessionPageState(page),
  }
}

export function externalProviderSessionPageSessions<T extends ExternalProviderSessionRecord>(
  page: ExternalProviderSessionPageLike<T> | null | undefined,
): T[] {
  return externalProviderSessionsSorted(page?.externalProviderSessions ?? page?.sessions)
}

export function externalProviderSessionPageState(
  page: ExternalProviderSessionPageLike | null | undefined,
): ExternalProviderSessionPageState {
  return {
    hasMore: externalProviderSessionPageHasMore(page),
    nextCursor: externalProviderSessionPageNextCursor(page),
  }
}

export function externalProviderSessionPageHasMore(
  page: ExternalProviderSessionPageLike | null | undefined,
): boolean {
  return page?.hasMore ?? page?.has_more ?? page?.externalProviderSessionsHasMore ?? false
}

export function externalProviderSessionPageNextCursor(
  page: ExternalProviderSessionPageLike | null | undefined,
): string | null {
  return page?.nextCursor ?? page?.next_cursor ?? page?.externalProviderSessionsNextCursor ?? null
}

export function mergeExternalProviderSessions<T extends ExternalProviderSessionRecord>(
  ...groups: readonly (readonly T[])[]
): T[] {
  const sessionsById = new Map<string, T>()
  for (const group of groups) {
    for (const session of group) {
      const existing = sessionsById.get(session.external_session_id)
      if (!existing) {
        sessionsById.set(session.external_session_id, session)
      } else {
        sessionsById.set(session.external_session_id, mergeExternalProviderSessionRecord(existing, session))
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
  return typeof session.last_modified_at_ms === "number" && Number.isFinite(session.last_modified_at_ms)
    ? session.last_modified_at_ms
    : 0
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

function mergeExternalProviderSessionRecord<T extends ExternalProviderSessionRecord>(
  existing: T,
  incoming: T,
): T {
  const existingModifiedMs = externalProviderSessionModifiedMs(existing)
  const incomingModifiedMs = externalProviderSessionModifiedMs(incoming)
  const primary = incomingModifiedMs > existingModifiedMs ? incoming : existing
  const fallback = primary === incoming ? existing : incoming
  return {
    ...primary,
    provider: nonBlankString(primary.provider) ?? nonBlankString(fallback.provider) ?? primary.provider,
    provider_session_id: nonBlankString(primary.provider_session_id)
      ?? nonBlankString(fallback.provider_session_id)
      ?? primary.provider_session_id,
    title: meaningfulOptionalString(primary.title) ?? meaningfulOptionalString(fallback.title) ?? primary.title ?? fallback.title,
    title_source: meaningfulOptionalString(primary.title_source)
      ?? meaningfulOptionalString(fallback.title_source)
      ?? primary.title_source
      ?? fallback.title_source,
    first_prompt_preview: meaningfulOptionalString(primary.first_prompt_preview)
      ?? meaningfulOptionalString(fallback.first_prompt_preview)
      ?? primary.first_prompt_preview
      ?? fallback.first_prompt_preview,
    created_at_ms: finiteNumber(primary.created_at_ms) ?? finiteNumber(fallback.created_at_ms) ?? primary.created_at_ms ?? fallback.created_at_ms,
    worktree_path: meaningfulOptionalString(primary.worktree_path)
      ?? meaningfulOptionalString(fallback.worktree_path)
      ?? primary.worktree_path
      ?? fallback.worktree_path,
    account_profile: meaningfulOptionalString(primary.account_profile)
      ?? meaningfulOptionalString(fallback.account_profile)
      ?? primary.account_profile
      ?? fallback.account_profile,
    capabilities: mergeExternalProviderSessionCapabilities(primary.capabilities, fallback.capabilities),
  } as T
}

function mergeExternalProviderSessionCapabilities(
  primary: ExternalProviderSessionCapabilities | undefined,
  fallback: ExternalProviderSessionCapabilities | undefined,
): ExternalProviderSessionCapabilities | undefined {
  if (!primary) {
    return fallback
  }
  if (!fallback) {
    return primary
  }
  const canReadHistory = primary.can_read_history ?? fallback.can_read_history
  if (canReadHistory === undefined) {
    return {
      ...fallback,
      ...primary,
    }
  }
  return {
    ...fallback,
    ...primary,
    can_read_history: canReadHistory,
  }
}

function meaningfulOptionalString(value: string | null | undefined): string | null | undefined {
  return value?.trim() ? value : undefined
}

function nonBlankString(value: string): string | undefined {
  return value.trim() ? value : undefined
}

function finiteNumber(value: number | null | undefined): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined
}

function clampExternalProviderSessionIndex(index: number, length: number): number {
  if (length <= 0) {
    return 0
  }
  const finiteIndex = Number.isFinite(index) ? Math.floor(index) : 0
  return Math.max(0, Math.min(finiteIndex, length - 1))
}
