import type { AgentInstance, RuntimeSession } from "./kernel-types.js"
import { sessionFocusedAgentId } from "./session-runtime-transition.js"

export type ResolvedAgentReference = {
  agent: AgentInstance | null
  error?: string | null
}

export function resolveSessionAgentReference(
  session: RuntimeSession,
  focusedAgentId: string | null | undefined,
  reference?: string | null,
): ResolvedAgentReference {
  const normalizedReference = reference?.trim() ?? ""
  if (!normalizedReference) {
    const resolvedFocusedAgentId = sessionFocusedAgentId({
      agents: session.agents,
      focused_agent_id: focusedAgentId ?? null,
    })
    const agent = resolvedFocusedAgentId
      ? session.agents.find((entry) => entry.id === resolvedFocusedAgentId) ?? null
      : null
    return agent
      ? { agent, error: null }
      : { agent: null, error: "no focused agent available" }
  }

  const matches = session.agents.filter((agent) => {
    return agent.id === normalizedReference
      || agent.agent_ref === normalizedReference
      || agent.alias === normalizedReference
  })
  if (matches.length === 1) {
    return { agent: matches[0]!, error: null }
  }
  if (matches.length > 1) {
    return { agent: null, error: `multiple agents match '${normalizedReference}'` }
  }
  return { agent: null, error: `agent '${normalizedReference}' not found` }
}
