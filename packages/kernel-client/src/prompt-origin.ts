export const ARROBA_PROMPT_ORIGIN = "arroba"
export const EXTERNAL_PROMPT_ORIGIN = "external"

export function normalizePromptOrigin(origin: string | null | undefined): string | null {
  const normalized = origin?.trim().toLowerCase()
  return normalized || null
}

export function promptOriginFromRecord(
  record: PromptOriginRecord | null | undefined,
  fallback?: string | null,
): string | null {
  return normalizePromptOrigin(record?.prompt_origin)
    ?? normalizePromptOrigin(fallback)
}

export function promptOriginFromPromptRecord(
  record: PromptOriginPromptRecord | null | undefined,
  fallback?: string | null,
): string | null {
  return promptOriginFromRecord(record, fallback)
    ?? (promptRecordHasExternalProviderIdentity(record) ? EXTERNAL_PROMPT_ORIGIN : null)
}

export function promptOriginIsExternal(origin: string | null | undefined): boolean {
  return normalizePromptOrigin(origin) === EXTERNAL_PROMPT_ORIGIN
}

export type PromptOriginRecord = {
  readonly prompt_origin?: string | null | undefined
}

export type PromptOriginPromptRecord = PromptOriginRecord & {
  readonly id?: string | null | undefined
  readonly external_provider?: string | null | undefined
  readonly external_provider_session_id?: string | null | undefined
  readonly external_provider_turn_id?: string | null | undefined
}

function promptRecordHasExternalProviderIdentity(record: PromptOriginPromptRecord | null | undefined): boolean {
  if (!record) {
    return false
  }
  return promptIdIsExternalProviderObserved(record.id)
    || nonBlankString(record.external_provider) !== null
    || nonBlankString(record.external_provider_session_id) !== null
    || nonBlankString(record.external_provider_turn_id) !== null
}

function promptIdIsExternalProviderObserved(value: string | null | undefined): boolean {
  const parts = value?.split(":")
  if (!parts || parts.length < 4 || parts[0] !== "external") {
    return false
  }
  return Boolean(parts[1]?.trim() && parts[2]?.trim() && parts.slice(3).join(":").trim())
}

function nonBlankString(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
}
