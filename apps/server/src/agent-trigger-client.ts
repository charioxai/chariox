import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { AgentTriggerClient as GatewayClient } from "@chariox/agent-trigger-gateway"
export { agentTriggerHistoryEntries } from "@chariox/agent-trigger-gateway"
export type { Invocation, Conversation } from "@chariox/agent-trigger-gateway"
export class AgentTriggerClient extends GatewayClient {
 constructor(sessionId: string, triggerId: string, endpoint: string) {
  super(sessionId, triggerId, new LocalIpcClient(endpoint, { controlRequestRetryDeadlineMs: 5000 }))
 }
}
