export const ARROBA_PROMPT_ORIGIN = "arroba"
export const EXTERNAL_PROMPT_ORIGIN = "external"

export function normalizePromptOrigin(origin: string | null | undefined): string | null {
  const normalized = origin?.trim().toLowerCase()
  return normalized || null
}

export function promptOriginIsExternal(origin: string | null | undefined): boolean {
  return normalizePromptOrigin(origin) === EXTERNAL_PROMPT_ORIGIN
}
