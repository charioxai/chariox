import type { RuntimeSession } from "./kernel-types.js"
import { attachToSessionRequest } from "./ipc-requests.js"
import { normalizeRuntimeSessionWithAgentActivity } from "./runtime-session-normalization.js"
import type { ShellContext } from "./shell-core.js"

type ShellSessionAttachmentDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
  clientId?: string | undefined
}

export function resolveShellAttachmentId(
  context: ShellContext,
): { ok: true; attachmentId: string } | { ok: false; message: string } {
  if (context.attachmentId) {
    return { ok: true, attachmentId: context.attachmentId }
  }
  return { ok: false, message: "open the Room with `session use <ref>` before running this command" }
}

export async function attachShellSession(sessionId: string, deps: ShellSessionAttachmentDeps): Promise<string | undefined> {
  if (!deps.clientId) {
    return undefined
  }
  const response = await deps.client.send(attachToSessionRequest(sessionId, deps.clientId))
  const payload = expectVariant<{ attachment: { id: string } }>(response, "SessionAttached")
  return payload.attachment.id
}

export function expectSessionState(response: Record<string, unknown>): RuntimeSession {
  if ("SessionState" in response) {
    const payload = response.SessionState as {
      session: RuntimeSession
      agent_activity?: RuntimeSession["agent_activity"] | null
      agent_activity_revision?: number | null
    }
    return normalizeRuntimeSessionWithAgentActivity(payload)
  }
  const payload = expectVariant<{
    session: RuntimeSession
    agent_activity?: RuntimeSession["agent_activity"] | null
    agent_activity_revision?: number | null
  }>(response, "SessionStateLoaded")
  return normalizeRuntimeSessionWithAgentActivity(payload)
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
