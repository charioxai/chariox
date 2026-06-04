export function homeExtensionAuditRecoveryAction(kind: string, payload: Record<string, unknown>): string | null {
  const status = typeof payload.status === "string" ? payload.status : ""
  const error = typeof payload.error === "string" ? payload.error.toLowerCase() : ""
  const agentRef = homeExtensionAuditAgentRef(payload)
  if (status === "replayed" || kind.includes(".replayed")) {
    return "cached idempotent result was returned; no retry needed"
  }
  if (status === "denied" || kind.includes(".denied")) {
    if (/worker|lease|provider run|run|stale|mismatch/.test(error)) {
      if (isConcreteHomeExtensionAuditAgentRef(agentRef)) {
        return `run /extension sync-status ${agentRef}; inspect /agent inspect ${agentRef}; retry only after the worker lease and provider run match the current home grant`
      }
      return "identify the affected agent in /kernel remote-runtime or the home extension audit, then retry only after the worker lease and provider run match the current home grant"
    }
    return "verify the home grant, safety limit, and caller authority before retrying"
  }
  if (status === "timeout" || kind.includes(".timeout")) {
    return "split the tool work or increase the home extension timeout before retrying"
  }
  if (status === "cancelled" || kind.includes(".cancel")) {
    return "retry only if the provider turn still needs this tool result"
  }
  if (status === "failed" || kind.includes(".failed")) {
    return "inspect the home-side tool configuration and logs, then retry"
  }
  return null
}

export function homeExtensionAuditAgentRef(payload: Record<string, unknown>): string {
  for (const key of ["agent_ref", "agent_id", "home_agent_id"]) {
    const value = payload[key]
    if (typeof value === "string" && isConcreteHomeExtensionAuditAgentRef(value.trim())) {
      return value.trim()
    }
  }
  return "affected agent"
}

function isConcreteHomeExtensionAuditAgentRef(agentRef: string): boolean {
  return Boolean(agentRef) && agentRef !== "affected agent" && !agentRef.startsWith("<")
}
