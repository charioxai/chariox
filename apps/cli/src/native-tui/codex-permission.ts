import { LocalIpcClient } from "../ipc.js"
import { resolveActiveNativePermissionInteraction } from "./native-permission-interaction.js"

type CodexPermissionResponseMessage = {
  result?: Record<string, unknown>
}

export async function resolveCodexNativePermissionResponse(
  message: CodexPermissionResponseMessage,
  options: {
    client: LocalIpcClient
    sessionId: string
    agentId: string
  },
): Promise<boolean> {
  const choiceId = codexNativePermissionChoice(message)
  if (!choiceId) return false
  return resolveActiveNativePermissionInteraction(options.client, options.sessionId, options.agentId, choiceId)
}

export function codexNativePermissionChoice(message: CodexPermissionResponseMessage): string | null {
  const result = message.result && typeof message.result === "object" ? message.result : null
  const decision = typeof result?.decision === "string" ? result.decision.toLowerCase() : ""
  const action = typeof result?.action === "string" ? result.action.toLowerCase() : ""
  const combined = `${decision} ${action}`
  if (combined.includes("decline") || combined.includes("deny") || combined.includes("reject")) {
    return "deny"
  }
  if (combined.includes("session") || combined.includes("always")) {
    return "allow_session"
  }
  if (combined.includes("accept") || combined.includes("allow")) {
    return "allow_once"
  }
  if (result && "permissions" in result) {
    return "allow_once"
  }
  return null
}
