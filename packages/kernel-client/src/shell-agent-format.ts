import type { AgentInstance } from "./kernel-types.js"

export function formatAgentRef(agent: AgentInstance): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

export function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => {
      const mode = agent.execution_mode_override ? ` mode=${agent.execution_mode_override}` : ""
      const permissions = agent.permission_level_override ? ` permissions=${agent.permission_level_override}` : ""
      return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]${mode}${permissions}`
    })
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}

export function formatAgentSubstituteSummary(agent: AgentInstance): string {
  const substitutes = agent.substitutes ?? []
  if (substitutes.length === 0) {
    return `${formatAgentRef(agent)} has no substitutes`
  }
  const lines = substitutes.map((substitute, index) => {
    const marker = agent.active_substitute_index === index ? "*" : "-"
    const variant = substitute.variant ? `/${substitute.variant}` : ""
    return `${marker} ${index}: ${substitute.provider}/${substitute.model}${variant}`
  })
  const timeout = agent.substitution_timeout_ms == null ? "default" : `${agent.substitution_timeout_ms}ms`
  return `${formatAgentRef(agent)} substitutes (${substitutes.length}, timeout ${timeout}):\n${lines.join("\n")}`
}
