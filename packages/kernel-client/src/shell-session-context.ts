import type { RuntimeSession } from "./kernel-types.js"

export function sessionContextAgentId(session: Pick<RuntimeSession, "agents" | "focused_agent_id">): string | undefined {
  const focusedAgentId = session.focused_agent_id?.trim()
  if (focusedAgentId && session.agents.some((agent) => agent.id === focusedAgentId)) {
    return focusedAgentId
  }
  return session.agents[0]?.id || undefined
}
