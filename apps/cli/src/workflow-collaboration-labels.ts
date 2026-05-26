import { formatAgentLabel } from "./agent-label.js"
import type { AgentInstance } from "./cli-types.js"

export const collaboratorNodeLabel = "collaborator node"
export const collaboratorAgentLabel = "another collaborator's agent"

export function workflowAgentDisplayLabel(agent: AgentInstance | null | undefined): string {
  return agent ? formatAgentLabel(agent) : collaboratorAgentLabel
}

export function workflowAgentRefDisplayLabel(agent: AgentInstance | null | undefined): string {
  return agent?.agent_ref ?? collaboratorAgentLabel
}
