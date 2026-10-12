import { LocalIpcClient } from "../ipc.js"
import { resolveActiveNativePermissionInteraction } from "./native-permission-interaction.js"

export async function resolveActiveOpenCodePermissionInteraction(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  choiceId: string,
  requestId?: string,
): Promise<boolean> {
  return resolveActiveNativePermissionInteraction(client, sessionId, agentId, choiceId,
    requestId === undefined ? undefined : `opencode-permission-${requestId}`)
}

export function openCodeNativePermissionChoice(body: Record<string, unknown>): string | null {
  const raw = [
    body.reply,
    body.response,
    body.decision,
    body.choice,
    body.action,
    body.status,
  ].find((value) => typeof value === "string")
  const value = typeof raw === "string" ? raw.toLowerCase() : ""
  if (["always", "session", "allow_session"].includes(value)) return "allow_session"
  if (["reject", "deny", "decline"].includes(value)) return "deny"
  if (["once", "allow_once", "allow", "approve", "approved"].includes(value)) return "allow_once"
  return null
}
