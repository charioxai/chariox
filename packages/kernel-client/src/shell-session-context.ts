import type { RuntimeSession } from "./kernel-types.js"
import { sessionFocusedAgentId } from "./session-runtime-transition.js"

export function sessionContextAgentId(session: Pick<RuntimeSession, "agents" | "focused_agent_id">): string | undefined {
  return sessionFocusedAgentId(session) ?? undefined
}
