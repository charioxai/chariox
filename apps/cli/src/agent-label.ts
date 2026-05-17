import type { AgentInstance } from "./cli-types.js"

export function formatAgentLabel(agent: AgentInstance | null | undefined) {
  if (!agent) {
    return ""
  }
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}
