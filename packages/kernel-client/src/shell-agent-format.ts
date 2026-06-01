import type { AgentInstance, SliceRecord } from "./kernel-types.js"

export function formatAgentRef(agent: AgentInstance): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

export function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => {
      const mode = agent.execution_mode_override ? ` mode=${agent.execution_mode_override}` : ""
      const permissions = agent.permission_level_override ? ` permissions=${agent.permission_level_override}` : ""
      return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]${mode}${permissions}`
    })
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}

export function formatAgentInspectSummary(
  agent: AgentInstance,
  slices: readonly SliceRecord[] = [],
  sliceLookupError?: string | null,
): string {
  const slice = sliceForRemoteAgent(agent, slices)
  const lines = [
    `${formatAgentRef(agent)} [${agent.state}]`,
    `id: ${agent.id}`,
    `session: ${agent.session_id}`,
    `provider: ${agent.provider}`,
    `model: ${agent.model ?? "<none>"}`,
    `variant: ${agent.effort ?? "<none>"}`,
    `mode: ${agent.execution_mode_override ?? "session"}`,
    `permissions: ${agent.permission_level_override ?? "session"}`,
    `workspace: ${agent.workspace_id ?? "<none>"}`,
    `worktree: ${agent.worktree_id ?? "<none>"}`,
    `placement: ${formatAgentPlacement(agent, slice)}`,
    ...(slice ? [
      `slice: ${formatSliceSummary(slice)}`,
      `slice provider accounts: ${formatSliceProviderAccounts(slice)}`,
    ] : sliceLookupError ? [
      `slice lookup: ${sliceLookupError}`,
    ] : []),
    `extensions: ${formatAgentExtensionSummary(agent)}`,
    `remote extension sync: ${formatAgentRemoteExtensionSyncSummary(agent)}`,
    `substitutes: ${formatAgentSubstitutesInline(agent)}`,
  ]
  const activeSubstitute = agent.active_substitute_index
  if (activeSubstitute != null) {
    lines.push(`active substitute: ${activeSubstitute}`)
  }
  if (agent.last_substitution) {
    lines.push(`last substitution: ${agent.last_substitution.reason}`)
  }
  lines.push(`created: ${formatTimestamp(agent.created_at_ms)}`)
  lines.push(`last activity: ${formatTimestamp(agent.last_activity_at_ms)}`)
  return lines.join("\n")
}

export function formatAgentSubstituteSummary(agent: AgentInstance): string {
  const substitutes = agent.substitutes ?? []
  if (substitutes.length === 0) {
    return `${formatAgentRef(agent)} has no substitutes`
  }
  const lines = substitutes.map((substitute, index) => {
    const marker = agent.active_substitute_index === index ? "*" : "-"
    const variant = substitute.variant ? `/${substitute.variant}` : ""
    return `${marker} ${index}: ${substitute.provider}/${substitute.model}${variant}`
  })
  const timeout = agent.substitution_timeout_ms == null ? "default" : `${agent.substitution_timeout_ms}ms`
  return `${formatAgentRef(agent)} substitutes (${substitutes.length}, timeout ${timeout}):\n${lines.join("\n")}`
}

function formatAgentPlacement(agent: AgentInstance, slice: SliceRecord | null = null): string {
  const remote = agent.remote_execution
  if (!remote) {
    return "worker-local"
  }
  const worker = remote.worker_machine_id || remote.worker_kernel_id
  const placement = slice ? `slice ${slice.name || slice.id}` : "remote"
  const parts = [
    worker ? `worker=${worker}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `active_run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return `${placement}${parts.length > 0 ? ` (${parts.join(", ")})` : ""}`
}

function formatAgentExtensionSummary(agent: AgentInstance): string {
  const grants = agent.extension_grants ?? []
  if (grants.length === 0) {
    return "none"
  }
  const counts = grants.reduce<Record<string, number>>((acc, grant) => {
    acc[grant.kind] = (acc[grant.kind] ?? 0) + 1
    return acc
  }, {})
  const byKind = ["mcp", "skill", "script", "connector"]
    .map((kind) => counts[kind] ? `${kind}=${counts[kind]}` : null)
    .filter(Boolean)
    .join(", ")
  const placement = agent.remote_execution ? "home-proxy/passive-snapshot" : "worker-local"
  return `${grants.length} grant${grants.length === 1 ? "" : "s"} (${placement}${byKind ? `; ${byKind}` : ""})`
}

function formatAgentRemoteExtensionSyncSummary(agent: AgentInstance): string {
  if (!agent.remote_execution) {
    return "not applicable"
  }
  const status = agent.remote_extension_manifest_sync
  if (!status) {
    return "pending"
  }
  const details = [
    status.state,
    status.pending_revoke ? "pending revoke" : null,
    status.manifest_hash ? `hash=${status.manifest_hash.slice(0, 12)}` : null,
    status.last_error ? `error=${status.last_error}` : null,
    status.last_synced_at_ms ? `synced=${formatTimestamp(status.last_synced_at_ms)}` : null,
    status.last_attempted_at_ms ? `attempted=${formatTimestamp(status.last_attempted_at_ms)}` : null,
  ].filter(Boolean)
  return details.join(", ")
}

function formatAgentSubstitutesInline(agent: AgentInstance): string {
  const substitutes = agent.substitutes ?? []
  if (substitutes.length === 0) {
    return "none"
  }
  return substitutes.map((substitute, index) => {
    const marker = agent.active_substitute_index === index ? "*" : ""
    const variant = substitute.variant ? `/${substitute.variant}` : ""
    return `${marker}${index}:${substitute.provider}/${substitute.model}${variant}`
  }).join(", ")
}

function formatTimestamp(timestampMs: number | null | undefined): string {
  if (!timestampMs) {
    return "<none>"
  }
  return new Date(timestampMs).toISOString()
}

function sliceForRemoteAgent(
  agent: AgentInstance,
  slices: readonly SliceRecord[],
): SliceRecord | null {
  const remote = agent.remote_execution
  if (!remote) {
    return null
  }
  return slices.find((slice) =>
    Boolean(slice.agent_ids?.includes(agent.id))
    || slice.worker_kernel_id === remote.worker_kernel_id
    || slice.worker_kernel_ref === remote.worker_kernel_id
    || slice.worker_machine_id === remote.worker_machine_id,
  ) ?? null
}

function formatSliceSummary(slice: SliceRecord): string {
  const worktree = slice.worktree_id || slice.workspace_mount || slice.workspace_id || "-"
  return `${slice.name || slice.id} (${[
    `id=${slice.id}`,
    `status=${slice.status}`,
    `display=${slice.display_mode ?? "headless"}`,
    `worktree=${worktree}`,
    `agents=${slice.agent_ids?.length ?? 0}`,
  ].join(", ")})`
}

function formatSliceProviderAccounts(slice: SliceRecord): string {
  const accounts = slice.provider_auth ?? []
  if (accounts.length === 0) {
    return "none"
  }
  return accounts.map((auth) => `${auth.provider}=${formatSliceAuthIdentity(auth)}`).join(", ")
}

function formatSliceAuthIdentity(auth: NonNullable<SliceRecord["provider_auth"]>[number]): string {
  const identity = auth.email || auth.account_id || auth.auth_type || auth.state
  if (auth.alias && auth.alias !== identity) {
    return `${auth.alias} (${identity})`
  }
  return identity
}
