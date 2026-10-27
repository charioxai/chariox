export const CHARIOX_PROMPT_ORIGIN = "chariox"
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
}
