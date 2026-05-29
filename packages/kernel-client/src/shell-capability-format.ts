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
  const sync = agent.remote_execution
    ? `\n\nremote extension sync: ${formatRemoteExtensionSyncStatusLine(agent.remote_extension_manifest_sync)}`
    : ""
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

export function formatRemoteExtensionSyncStatus(agent: AgentInstance): string {
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (!agent.remote_execution) {
    return `${agentLabel} is worker-local; no home-proxy manifest is projected.`
  }
  const status = agent.remote_extension_manifest_sync
  const rows = [
    `${agentLabel} remote extension sync: ${formatRemoteExtensionSyncStatusLine(status)}`,
    `worker kernel: ${agent.remote_execution.worker_kernel_id}`,
    `worker machine: ${agent.remote_execution.worker_machine_id}`,
    `leased agent: ${agent.remote_execution.leased_agent_id}`,
    `active worker run: ${agent.remote_execution.active_worker_provider_run_id ?? "none"}`,
  ]
  if (status?.manifest_hash) rows.push(`manifest hash: ${status.manifest_hash}`)
  if (status?.last_synced_at_ms) rows.push(`last synced: ${new Date(status.last_synced_at_ms).toISOString()}`)
  if (status?.last_attempted_at_ms) rows.push(`last attempted: ${new Date(status.last_attempted_at_ms).toISOString()}`)
  if (status?.last_error) rows.push(`last error: ${status.last_error}`)
  if (status?.pending_revoke) rows.push("revoke state: pending worker acknowledgement")
  return rows.join("\n")
}

export function formatHomeExtensionAuditEvents(events: readonly Record<string, unknown>[]): string {
  if (events.length === 0) {
    return "no home extension audit events"
  }
  return events.map((event) => {
    const payload = typeof event.payload === "object" && event.payload ? event.payload as Record<string, unknown> : {}
    const tool = typeof payload.tool === "object" && payload.tool ? payload.tool as Record<string, unknown> : {}
    const status = typeof payload.status === "string" ? ` ${payload.status}` : ""
    const name = typeof tool.tool_name === "string" ? ` ${tool.tool_name}` : ""
    const at = typeof event.timestamp_ms === "number" ? new Date(event.timestamp_ms).toISOString() : "unknown-time"
    return `${at} ${String(event.kind ?? "event")}${name}${status}`
  }).join("\n")
}

function formatRemoteExtensionSyncStatusLine(status?: RemoteExtensionManifestSyncStatus | null): string {
  if (!status) return "pending"
  const revoke = status.pending_revoke ? ", pending revoke" : ""
  const error = status.last_error ? `, ${status.last_error}` : ""
  return `${status.state}${revoke}${error}`
}
