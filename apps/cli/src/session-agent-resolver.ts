import type { AgentInstance, RuntimeSession } from "./cli-types.js"

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
    const agent = focusedAgentId
      ? session.agents.find((entry) => entry.id === focusedAgentId) ?? null
      : session.agents[0] ?? null
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
