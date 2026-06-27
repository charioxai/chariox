import { normalizeRuntimeSessionWithAgentActivity, type RuntimeSession } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  getSessionStateRequest,
  respondToInteractionRequest,
} from "../ipc-requests.js"

export async function resolveActiveNativePermissionInteraction(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  choiceId: string,
): Promise<boolean> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  const session = normalizeRuntimeSessionWithAgentActivity(
    expectVariant<{
      session: RuntimeSession
      agent_activity?: RuntimeSession["agent_activity"] | null
      agent_activity_revision?: number | null
    }>(response, "SessionState"),
  )
  const interaction = session.active_interactions?.find((entry) =>
    entry.agent_id === agentId && entry.kind === "permission")
  if (!interaction) return false
  await client.send<Record<string, unknown>>(
    respondToInteractionRequest(sessionId, interaction.id, choiceId),
  )
  return true
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
