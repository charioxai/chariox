import type { AgentInstance } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { revokeAgentExtensionRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function revokeAgentApp(client: LocalIpcClient, agentRef: string, installationId: string): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "app", installationId))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
}
