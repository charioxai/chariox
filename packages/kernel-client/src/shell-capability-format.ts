import type {
  AgentInstance,
  RemoteExtensionManifestSyncStatus,
  ArrobaEnvironmentConfig,
  ArrobaMcpServerConfig,
  ArrobaScriptMetadata,
  ArrobaSkillMetadata,
  ExtensionKind,
  McpImportOutcome,
  SkillImportOutcome,
} from "./kernel-types.js"

export function formatMcpList(mcps: ArrobaMcpServerConfig[]): string {
  if (mcps.length === 0) {
    return "no MCP servers installed"
  }
  return mcps.map((mcp) => {
    const enabled = mcp.enabled === false ? "disabled" : "enabled"
    const transport = Object.keys(mcp.transport ?? {})[0] ?? "transport"
    return `${mcp.name} [${enabled}] ${transport}`
  }).join("\n")
}

export function formatSkillList(skills: ArrobaSkillMetadata[]): string {
  if (skills.length === 0) {
    return "no skills installed"
  }
  return skills.map((skill) => {
    const description = skill.short_description || skill.description || skill.path
    return `${skill.name} - ${description}`
  }).join("\n")
}

export function formatMcpImportOutcome(outcome: McpImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported MCPs: ${outcome.imported.map((mcp) => mcp.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped MCPs:")
    lines.push(...outcome.skipped.map((skip) => `- ${skip.name}: ${skip.reason}`))
  }
  return lines.length === 0 ? "No MCPs imported." : lines.join("\n")
}

export function formatSkillImportOutcome(outcome: SkillImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported skills: ${outcome.imported.map((skill) => skill.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped skills:")
    lines.push(...outcome.skipped.map((skip) => `- ${skip.name}: ${skip.reason}`))
  }
  return lines.length === 0 ? "No skills imported." : lines.join("\n")
}

export function formatEnvironmentList(environments: ArrobaEnvironmentConfig[]): string {
  if (environments.length === 0) {
    return "no environments registered"
  }
  return environments.map((environment) => {
    const runtime = typeof environment.runtime?.type === "string"
      ? environment.runtime.type
      : Object.keys(environment.runtime ?? {})[0] ?? "runtime"
    return `${environment.name} [${runtime}]`
  }).join("\n")
}

export function formatScriptList(scripts: ArrobaScriptMetadata[]): string {
  if (scripts.length === 0) {
    return "no scripts registered"
  }
  return scripts.map((script) => `${script.name} [${script.runtime}] - ${script.description}`).join("\n")
}

export function formatAgentExtensionGrants(agent: AgentInstance, kind: ExtensionKind): string {
  const grants = (agent.extension_grants ?? []).filter((grant) => grant.kind === kind)
  const label = kind === "mcp" ? "MCP" : kind
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (grants.length === 0) {
    return `${agentLabel} has no ${label} grants.`
  }
  const placement = agent.remote_execution
    ? kind === "skill" ? "skill snapshot" : "home-proxy"
    : "worker-local"
  const sync = formatGrantRemoteExtensionSyncBlock(agent)
  return `${agentLabel} ${label} grants:\n${grants.map((grant) => {
    const parts = [
      placement,
      grant.environment ? `env=${grant.environment}` : null,
      grant.credential ? `credential=${grant.credential}` : null,
      grant.max_safety ? `allow=${grant.max_safety}` : null,
    ].filter(Boolean)
    const suffix = parts.length > 0 ? ` (${parts.join(", ")})` : ""
    return `- ${grant.name}${suffix}`
  }).join("\n")}${sync}`
}

function formatGrantRemoteExtensionSyncBlock(agent: AgentInstance): string {
  if (!agent.remote_execution) return ""
  const status = agent.remote_extension_manifest_sync
  const lines = [
    `remote extension sync: ${formatRemoteExtensionSyncStatusLine(status)}`,
    `placement: ${formatRemoteExtensionPlacement(agent.remote_execution)}`,
  ]
  const nextAction = remoteExtensionSyncNextAction(
    status,
    agent.agent_ref,
    agent.remote_execution.worker_machine_id,
  )
  if (nextAction) lines.push(`next: ${nextAction}`)
  return `\n\n${lines.join("\n")}`
}

export function formatRemoteExtensionSyncStatus(agent: AgentInstance): string {
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (!agent.remote_execution) {
    return `${agentLabel} is worker-local; no home-proxy manifest is projected.`
  }
  const status = agent.remote_extension_manifest_sync
  const rows = [
    `${agentLabel} remote extension sync: ${formatRemoteExtensionSyncStatusLine(status)}`,
    `placement: ${formatRemoteExtensionPlacement(agent.remote_execution)}`,
    `worker kernel: ${agent.remote_execution.worker_kernel_id}`,
    `worker machine: ${agent.remote_execution.worker_machine_id}`,
    `execution lease: ${agent.remote_execution.execution_lease_id}`,
    `leased agent: ${agent.remote_execution.leased_agent_id}`,
    `active worker run: ${agent.remote_execution.active_worker_provider_run_id ?? "none"}`,
  ]
  if (status?.manifest_hash) rows.push(`manifest hash: ${status.manifest_hash}`)
  if (status?.last_synced_at_ms) rows.push(`last synced: ${new Date(status.last_synced_at_ms).toISOString()}`)
  if (status?.last_attempted_at_ms) rows.push(`last attempted: ${new Date(status.last_attempted_at_ms).toISOString()}`)
  if (status?.last_error) rows.push(`last error: ${status.last_error}`)
  if (status?.pending_revoke) rows.push("revoke state: pending worker acknowledgement")
  const nextAction = remoteExtensionSyncNextAction(
    status,
    agent.agent_ref,
    agent.remote_execution.worker_machine_id,
  )
  if (nextAction) rows.push(`next: ${nextAction}`)
  return rows.join("\n")
}

function formatRemoteExtensionPlacement(remote: NonNullable<AgentInstance["remote_execution"]>): string {
  const parts = [
    remote.worker_machine_id ? `worker=${remote.worker_machine_id}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `active_run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return parts.length > 0 ? `remote (${parts.join(", ")})` : "remote"
}

export function formatHomeExtensionAuditEvents(events: readonly Record<string, unknown>[]): string {
  if (events.length === 0) {
    return "no home extension audit events"
  }
  return events.map((event) => {
    const payload = typeof event.payload === "object" && event.payload ? event.payload as Record<string, unknown> : {}
    const tool = typeof payload.tool === "object" && payload.tool ? payload.tool as Record<string, unknown> : {}
    const grant = typeof payload.grant === "object" && payload.grant ? payload.grant as Record<string, unknown> : {}
    const status = typeof payload.status === "string" ? ` ${payload.status}` : ""
    const name = typeof tool.tool_name === "string"
      ? ` ${tool.tool_name}`
      : typeof grant.name === "string" ? ` ${grant.name}` : ""
    const at = typeof event.timestamp_ms === "number" ? new Date(event.timestamp_ms).toISOString() : "unknown-time"
    const rows = [`${at} ${String(event.kind ?? "event")}${name}${status}`]
    const actor = [
      fieldPart("home", payload.home_user_id),
      fieldPart("caller", payload.caller_user_id),
      fieldPart("agent", payload.agent_id ?? payload.home_agent_id),
      fieldPart("lease", payload.lease_id),
      fieldPart("worker", payload.worker_kernel_id),
      fieldPart("run", payload.worker_provider_run_id ?? payload.active_worker_provider_run_id),
    ].filter(Boolean)
    if (actor.length > 0) rows.push(`  actor: ${actor.join(" ")}`)
    if (Object.keys(tool).length > 0) {
      const details = [
        typeof tool.kind === "string" && typeof tool.name === "string" ? `${tool.kind}:${tool.name}` : null,
        fieldPart("as", tool.tool_name),
        fieldPart("safety", tool.safety),
        fieldPart("timeout", typeof tool.timeout_sec === "number" ? `${tool.timeout_sec}s` : tool.timeout_sec),
        fieldPart("hash", tool.version_hash),
      ].filter(Boolean)
      if (details.length > 0) rows.push(`  tool: ${details.join(" ")}`)
    }
    if (Object.keys(grant).length > 0) {
      const details = [
        typeof grant.kind === "string" && typeof grant.name === "string" ? `${grant.kind}:${grant.name}` : null,
        fieldPart("env", grant.environment),
        typeof grant.credential_present === "boolean" ? `credential=${grant.credential_present ? "yes" : "no"}` : null,
        fieldPart("allow", grant.max_safety),
      ].filter(Boolean)
      if (details.length > 0) rows.push(`  grant: ${details.join(" ")}`)
    }
    const invocation = formatHomeExtensionAuditInvocation(payload.invocation)
    if (invocation) rows.push(`  invocation: ${invocation}`)
    const result = [
      fieldPart("ok", payload.ok),
      fieldPart("bytes", payload.result_bytes),
      fieldPart("duration", typeof payload.duration_ms === "number" ? `${payload.duration_ms}ms` : payload.duration_ms),
    ].filter(Boolean)
    if (result.length > 0) rows.push(`  result: ${result.join(" ")}`)
    if (typeof payload.error === "string" && payload.error) rows.push(`  error: ${payload.error}`)
    const next = homeExtensionAuditNextAction(String(event.kind ?? ""), payload)
    if (next) rows.push(`  next: ${next}`)
    return rows.join("\n")
  }).join("\n")
}

function formatHomeExtensionAuditInvocation(value: unknown): string {
  const invocation = typeof value === "object" && value ? value as Record<string, unknown> : {}
  const parts = [
    fieldPart("id", invocation.invocation_id),
    fieldPart("call", invocation.provider_tool_call_id),
    fieldPart("attempt", invocation.attempt),
    fieldPart("idempotency", invocation.idempotency_key),
  ].filter(Boolean)
  return parts.join(" ")
}

function homeExtensionAuditNextAction(kind: string, payload: Record<string, unknown>): string {
  const status = typeof payload.status === "string" ? payload.status : ""
  const error = typeof payload.error === "string" ? payload.error.toLowerCase() : ""
  const agentRef = auditAgentRef(payload)
  if (status === "replayed" || kind.includes(".replayed")) {
    return "cached idempotent result was returned; no retry needed"
  }
  if (status === "denied" || kind.includes(".denied")) {
    if (/worker|lease|provider run|run|stale|mismatch/.test(error)) {
      return `run /extension sync-status ${agentRef}; use /extension sync-retry ${agentRef} after the worker/provider run is current`
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
  return ""
}

function auditAgentRef(payload: Record<string, unknown>): string {
  for (const key of ["agent_ref", "agent_id", "home_agent_id"]) {
    const value = payload[key]
    if (typeof value === "string" && value.trim()) {
      return value.trim()
    }
  }
  return "<agent>"
}

function formatRemoteExtensionSyncStatusLine(status?: RemoteExtensionManifestSyncStatus | null): string {
  if (!status) return "pending"
  const revoke = status.pending_revoke ? ", pending revoke" : ""
  const error = status.last_error ? `, ${status.last_error}` : ""
  return `${status.state}${revoke}${error}`
}

function remoteExtensionSyncNextAction(
  status: RemoteExtensionManifestSyncStatus | null | undefined,
  agentRef: string,
  workerMachineId?: string | null,
): string | null {
  if (!status || status.state === "pending" || status.state === "syncing") {
    return workerMachineId
      ? `wait for the worker manifest update; run /extension sync-status ${agentRef}; run /machine kernels ${workerMachineId} if it does not settle; use /extension sync-retry ${agentRef} after worker connectivity is healthy`
      : `wait for the worker manifest update; run /extension sync-status ${agentRef} if it does not settle; use /extension sync-retry ${agentRef} after worker connectivity is healthy`
  }
  if (status.pending_revoke) {
    return workerMachineId
      ? `keep the home revoke in place; run /machine kernels ${workerMachineId}; run /extension sync-retry ${agentRef} after the worker reconnects`
      : `keep the home revoke in place; run /extension sync-retry ${agentRef} after the worker reconnects`
  }
  if (status.state === "failed" || status.state === "stale") {
    return workerMachineId
      ? `run /machine kernels ${workerMachineId}, then run /extension sync-retry ${agentRef}`
      : `check worker connectivity, then run /extension sync-retry ${agentRef}`
  }
  return null
}

function fieldPart(label: string, value: unknown): string | null {
  if (value === null || value === undefined || value === "") return null
  if (typeof value === "number" && !Number.isFinite(value)) return null
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") return `${label}=${value}`
  return null
}
