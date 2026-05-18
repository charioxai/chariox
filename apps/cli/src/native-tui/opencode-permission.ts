import { type RuntimeSession } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  getSessionStateRequest,
  respondToInteractionRequest,
} from "../ipc-requests.js"

export async function resolveActiveOpenCodePermissionInteraction(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  choiceId: string,
): Promise<boolean> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
  const interaction = session.active_interactions?.find((entry) =>
    entry.agent_id === agentId && entry.kind === "permission")
  if (!interaction) return false
  await client.send<Record<string, unknown>>(
    respondToInteractionRequest(sessionId, interaction.id, choiceId),
  )
  return true
}

export function openCodeNativePermissionChoice(body: Record<string, unknown>): string {
  const raw = [
    body.response,
    body.decision,
    body.choice,
    body.action,
    body.status,
  ].find((value) => typeof value === "string")
  const value = typeof raw === "string" ? raw.toLowerCase() : ""
  if (value.includes("always") || value.includes("session")) return "allow_session"
  if (value.includes("reject") || value.includes("deny") || value.includes("decline")) return "deny"
  return "allow_once"
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
