export function homeExtensionAuditRecoveryAction(kind: string, payload: Record<string, unknown>): string | null {
  const status = typeof payload.status === "string" ? payload.status : ""
  const error = typeof payload.error === "string" ? payload.error.toLowerCase() : ""
  const agentRef = homeExtensionAuditAgentRef(payload)
  if (status === "replayed" || kind.includes(".replayed")) {
    return "cached idempotent result was returned; no retry needed"
  }
  if (kind.includes(".manifest.failed") || kind.includes(".manifest.retry_scheduled")) {
    const workerMachine = homeExtensionAuditWorkerMachine(payload)
    const pendingRevoke = payload.pending_revoke === true
    if (isConcreteHomeExtensionAuditAgentRef(agentRef)) {
      if (pendingRevoke) {
        return workerMachine
          ? `keep the home revoke in place; run /extension sync-status ${agentRef}; run /machine kernels ${workerMachine} if the revoke stays pending; use /extension sync-retry ${agentRef} after the worker reconnects`
          : `keep the home revoke in place; run /extension sync-status ${agentRef}; run /kernel remote-runtime to identify worker connectivity if the revoke stays pending; use /extension sync-retry ${agentRef} after the worker reconnects`
      }
      return workerMachine
        ? `home keeps stale home-proxy calls blocked; run /extension sync-status ${agentRef}; run /machine kernels ${workerMachine}; use /extension sync-retry ${agentRef} after worker connectivity is healthy`
        : `home keeps stale home-proxy calls blocked; run /extension sync-status ${agentRef}; use /extension sync-retry ${agentRef} after worker connectivity is healthy`
    }
    if (pendingRevoke) {
      return "keep the home revoke in place; identify the affected agent in /kernel remote-runtime, then retry manifest sync after the worker reconnects"
    }
    return "home keeps stale home-proxy calls blocked; identify the affected agent in /kernel remote-runtime, then retry manifest sync after worker connectivity is healthy"
  }
  if (kind.includes(".manifest.synced") && payload.revoke_acknowledged === true) {
    return "worker acknowledged the revoke; home will continue denying calls for the removed grant"
  }
  if (status === "denied" || kind.includes(".denied")) {
    if (/worker|lease|provider run|run|stale|mismatch/.test(error)) {
      if (isConcreteHomeExtensionAuditAgentRef(agentRef)) {
        const workerMachine = homeExtensionAuditWorkerMachine(payload)
        return workerMachine
          ? `run /extension sync-status ${agentRef}; run /machine kernels ${workerMachine}; inspect /agent inspect ${agentRef}; retry only after the worker lease and provider run match the current home grant`
          : `run /extension sync-status ${agentRef}; inspect /agent inspect ${agentRef}; retry only after the worker lease and provider run match the current home grant`
      }
      return "identify the affected agent in /kernel remote-runtime or the home extension audit, then retry only after the worker lease and provider run match the current home grant"
    }
    return "verify the home grant, safety limit, and caller authority before retrying"
  }
  if (status === "timeout" || kind.includes(".timeout")) {
    return "split the tool work or increase the home extension timeout before retrying"
  }
  if (status === "not_in_flight") {
    return "no matching in-flight home extension call was found; if the provider turn still waits for this tool, refresh /kernel remote-runtime and retry cancellation from the current turn"
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

function homeExtensionAuditWorkerMachine(payload: Record<string, unknown>): string | null {
  for (const key of ["worker_machine_id", "machine_id"]) {
    const value = payload[key]
    if (typeof value === "string") {
      const machine = value.trim()
      if (machine && !machine.startsWith("<")) {
        return machine
      }
    }
  }
  return null
}
