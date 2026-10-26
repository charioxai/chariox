// MP-08 / MP-10 / MP-11: the existing bundle is authoritative for an exact session ID.
import { LocalIpcError, type LocalIpcClient } from "./ipc.js"
import { DEFAULT_MAX_AGENTS_PER_SCREEN, resolveMaxAgentsPerScreen, type CharioxPreferences } from "./preferences.js"
import type { CliOptions } from "./cli-types.js"
import type { BootstrapDeps } from "./session-bootstrap.js"
type InitialView = Awaited<ReturnType<NonNullable<BootstrapDeps["attachSessionView"]>>>

export function requestInitialSessionView(client: LocalIpcClient, options: CliOptions, preferences: CharioxPreferences,
  attach: BootstrapDeps["attachSessionView"]): Promise<InitialView | null> | null {
  if (!attach || options.createSession || !options.sessionId || !/^[a-f0-9]{16}$/.test(options.sessionId)
    || resolveMaxAgentsPerScreen(preferences.ui?.maxAgentsPerScreen) !== DEFAULT_MAX_AGENTS_PER_SCREEN) return null
  return attach(client, options.sessionId, options.clientId, null).catch(error => {
    // A hexadecimal alias can still require the ordinary workspace-aware resolver.
    if (error instanceof LocalIpcError && error.code === "session_not_found") return null
    throw error
  })
}
