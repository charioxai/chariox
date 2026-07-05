import type { RuntimeSession } from "./kernel-types.js"

export const SESSION_CONFIG_RESPONSE_LAYOUT_KEY = "ui.multiAgentResponseLayout"

export type SessionResponseLayout = "individual" | "split"

export function sessionResponseLayout(
  session: Pick<RuntimeSession, "config_state"> | null | undefined,
  fallback?: SessionResponseLayout | string | null,
): SessionResponseLayout {
  return normalizeSessionResponseLayout(
    session?.config_state?.values?.[SESSION_CONFIG_RESPONSE_LAYOUT_KEY],
  )
    ?? normalizeSessionResponseLayout(fallback)
    ?? "individual"
}

export function normalizeSessionResponseLayout(
  value?: string | null,
): SessionResponseLayout | null {
  return value === "split" || value === "individual" ? value : null
}
