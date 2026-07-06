import type { RuntimeSession } from "./kernel-types.js"
import { normalizeRuntimeSessionWithAgentActivity } from "./runtime-session-normalization.js"

export function sessionWithProjectedAgentActivity(payload: {
  session: RuntimeSession
  agent_activity?: RuntimeSession["agent_activity"] | null | undefined
  agent_activity_revision?: number | null | undefined
}): RuntimeSession {
  return normalizeRuntimeSessionWithAgentActivity(payload)
}
