import type {
  AgentInstance,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { grantAgentCapabilityRequest } from "../ipc-requests.js"

export async function grantNativeCapabilities(
  client: LocalIpcClient,
  workspace: string,
  agentId: string,
  mcps: string[],
  skills: string[],
): Promise<void> {
  for (const name of mcps) {
    const response = await client.send<Record<string, unknown>>(grantAgentCapabilityRequest(workspace, agentId, "mcp", name))
    const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentCapabilityGranted").agent
    if (!agent.mcp_grants?.includes(name)) {
      throw new Error(`Arroba MCP grant ${name} was accepted but is missing from agent ${agentId}`)
    }
  }
  for (const name of skills) {
    const response = await client.send<Record<string, unknown>>(grantAgentCapabilityRequest(workspace, agentId, "skill", name))
    const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentCapabilityGranted").agent
    if (!agent.skill_grants?.includes(name)) {
      throw new Error(`Arroba skill grant ${name} was accepted but is missing from agent ${agentId}`)
    }
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
