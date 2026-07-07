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

export function promptOriginIsExternal(origin: string | null | undefined): boolean {
  return normalizePromptOrigin(origin) === EXTERNAL_PROMPT_ORIGIN
}

export type PromptOriginRecord = {
  readonly prompt_origin?: string | null | undefined
  readonly external_provider?: string | null | undefined
  readonly external_provider_session_id?: string | null | undefined
  readonly external_provider_turn_id?: string | null | undefined
}
