import { LocalIpcClient } from "../ipc.js"
import { resolveActiveNativePermissionInteraction } from "./native-permission-interaction.js"

export async function resolveActiveOpenCodePermissionInteraction(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  choiceId: string,
): Promise<boolean> {
  return resolveActiveNativePermissionInteraction(client, sessionId, agentId, choiceId)
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
