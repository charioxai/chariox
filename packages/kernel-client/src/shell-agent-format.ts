import type { AgentInstance, SliceRecord } from "./kernel-types.js"
import { remoteExtensionSyncNextAction } from "./shell-capability-format.js"
import {
  hasActiveHomeProxyExtensionGrants,
  shouldShowRemoteExtensionManifestSync,
} from "./extension-grant-placement.js"

export type ShellAgentProviderRunContext = {
  activeProviderRunId?: string | null
  activeProviderRunAgentId?: string | null
  activeProviderRunLookupError?: string | null
}

export type ShellAgentSessionContext = {
  homeKernelId?: string | null
  homeMachineId?: string | null
  ownerUserId?: string | null
  workspaceLiveSyncMode?: "managed" | "tracked" | "unrestricted" | null
}

export function formatAgentRef(agent: AgentInstance): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

export function formatAgentListSummary(
  agents: AgentInstance[],
  slices: readonly SliceRecord[] = [],
  providerRunContext: ShellAgentProviderRunContext = {},
): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => formatAgentListEntry(agent, sliceForRemoteAgent(agent, slices), providerRunContext))
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}

function formatAgentListEntry(
  agent: AgentInstance,
  slice: SliceRecord | null,
  providerRunContext: ShellAgentProviderRunContext,
): string {
  const parts = [
    agent.state,
    formatAgentProvider(agent),
    `worktree ${agent.worktree_id ?? "-"}`,
    formatAgentListPlacement(agent, slice),
    formatAgentListProviderRun(agent, providerRunContext),
    agent.execution_mode_override ? `mode ${agent.execution_mode_override}` : null,
    agent.permission_level_override ? `permissions ${agent.permission_level_override}` : null,
    formatAgentListGrantCount(agent),
    formatAgentListRemoteExtensionSync(agent),
  ].filter(Boolean)
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${parts.join("; ")}]`
}

function formatAgentListProviderRun(
  agent: AgentInstance,
  context: ShellAgentProviderRunContext,
): string | null {
  const runId = agentProviderRunId(agent, context)
  return runId ? `session run ${runId}` : null
}

function formatAgentProvider(agent: AgentInstance): string {
  const provider = agent.primary_provider ?? agent.provider
  const model = agent.primary_model ?? agent.model
  if (!model) {
    return provider
  }
  return model.startsWith(`${provider}/`) ? model : `${provider} ${model}`
}

function formatAgentListPlacement(agent: AgentInstance, slice: SliceRecord | null): string {
  const remote = agent.remote_execution
  if (!remote) {
    return "local"
  }
  if (slice) {
    return `slice ${slice.name || slice.id}`
  }
  const machine = remote.worker_machine_id ? `@${remote.worker_machine_id}` : ""
  const run = remote.active_worker_provider_run_id ? ` run ${remote.active_worker_provider_run_id}` : ""
  return `remote ${remote.worker_kernel_id}${machine}${run}`
}

function formatAgentListGrantCount(agent: AgentInstance): string {
  const grants = agent.extension_grants?.length ?? 0
  const count = `${grants} grant${grants === 1 ? "" : "s"}`
  return grants > 0 ? `${count} (${formatAgentExtensionPlacementSummary(agent)})` : count
}

function formatAgentListRemoteExtensionSync(agent: AgentInstance): string | null {
  if (!agent.remote_execution) {
    return null
  }
  const status = agent.remote_extension_manifest_sync
  if (!shouldShowRemoteExtensionManifestSync(agent.extension_grants, status)) {
    return null
  }
  if (!status) {
    return `manifest pending next ${formatAgentListRemoteExtensionSyncAction(agent)}`
  }
  const hash = status.manifest_hash ? ` ${status.manifest_hash.slice(0, 8)}` : ""
  const revoke = status.pending_revoke ? " pending revoke" : ""
  const error = status.last_error ? ` error ${status.last_error}` : ""
  const action = status.state === "failed" || status.state === "stale" || status.pending_revoke || status.last_error
    ? ` next ${formatAgentListRemoteExtensionSyncAction(agent)}`
    : ""
  return `manifest ${status.state}${hash}${revoke}${error}${action}`
}

function formatAgentListRemoteExtensionSyncAction(agent: AgentInstance): string {
  return remoteExtensionSyncNextAction(
    agent.remote_extension_manifest_sync,
    agent.agent_ref,
    agent.remote_execution?.worker_machine_id,
  ) ?? `run /extension sync-status ${agent.agent_ref}`
}

export function formatAgentInspectSummary(
  agent: AgentInstance,
  slices: readonly SliceRecord[] = [],
  sliceLookupError?: string | null,
  providerRunContext: ShellAgentProviderRunContext = {},
  sessionContext: ShellAgentSessionContext = {},
): string {
  const slice = sliceForRemoteAgent(agent, slices)
  const lines = [
    `${formatAgentRef(agent)} [${agent.state}]`,
    `id: ${agent.id}`,
    `session: ${agent.session_id}`,
    `home kernel: ${formatHomeKernel(sessionContext)}`,
    `session owner: ${sessionContext.ownerUserId || "<unknown>"}`,
    `live sync: ${formatSessionWorkspaceLiveSyncMode(sessionContext.workspaceLiveSyncMode)}`,
    `provider: ${agent.provider}`,
    `model: ${agent.model ?? "<none>"}`,
    `variant: ${agent.effort ?? "<none>"}`,
    `mode: ${agent.execution_mode_override ?? "session"}`,
    `permissions: ${agent.permission_level_override ?? "session"}`,
    `workspace: ${agent.workspace_id ?? "<none>"}`,
    `worktree: ${agent.worktree_id ?? "<none>"}`,
    `placement: ${formatAgentPlacement(agent, slice)}`,
    `provider run: ${formatAgentProviderRunSummary(agent, providerRunContext)}`,
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

function formatHomeKernel(context: ShellAgentSessionContext): string {
  const homeKernel = context.homeKernelId || "<unknown>"
  return context.homeMachineId ? `${homeKernel}@${context.homeMachineId}` : homeKernel
}

function formatSessionWorkspaceLiveSyncMode(
  mode: ShellAgentSessionContext["workspaceLiveSyncMode"],
): string {
  if (mode === "managed" || mode === "tracked") {
    return `${mode} (selected workspace/worktree only; other repositories unrestricted)`
  }
  if (mode === "unrestricted") {
    return "off"
  }
  return "config default"
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

function formatAgentProviderRunSummary(
  agent: AgentInstance,
  context: ShellAgentProviderRunContext,
): string {
  const sessionRunId = agentProviderRunId(agent, context)
  const workerRunId = agent.remote_execution?.active_worker_provider_run_id ?? null
  if (!sessionRunId && context.activeProviderRunId && context.activeProviderRunAgentId && context.activeProviderRunAgentId !== agent.id) {
    const worker = workerRunId ? `, worker=${workerRunId}` : ""
    return `session=${context.activeProviderRunId} owned_by=${context.activeProviderRunAgentId}${worker}`
  }
  if (!sessionRunId && context.activeProviderRunId && !context.activeProviderRunAgentId) {
    const reason = context.activeProviderRunLookupError ? `; ${context.activeProviderRunLookupError}` : ""
    const worker = workerRunId ? `, worker=${workerRunId}` : ""
    return `session=${context.activeProviderRunId} owner unknown${reason}${worker}`
  }
  if (!sessionRunId && !workerRunId) {
    return "none"
  }
  return [
    sessionRunId ? `session=${sessionRunId}` : null,
    workerRunId ? `worker=${workerRunId}` : null,
  ].filter(Boolean).join(", ")
}

function agentProviderRunId(
  agent: AgentInstance,
  context: ShellAgentProviderRunContext,
): string | null {
  if (!context.activeProviderRunId) {
    return null
  }
  if (context.activeProviderRunAgentId) {
    return context.activeProviderRunAgentId === agent.id ? context.activeProviderRunId : null
  }
  return null
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
  const placement = formatAgentExtensionPlacementSummary(agent)
  return `${grants.length} grant${grants.length === 1 ? "" : "s"} (${placement}${byKind ? `; ${byKind}` : ""})`
}

export function formatAgentExtensionPlacementSummary(agent: AgentInstance): string {
  if (!agent.remote_execution) {
    return "worker-local"
  }
  const grants = agent.extension_grants ?? []
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(grants)
  const passiveSkillSnapshot = grants.some((grant) => grant.kind === "skill")
  return [
    activeHomeProxy ? "active tools home-proxy" : null,
    passiveSkillSnapshot ? "skills snapshot" : null,
  ].filter(Boolean).join("; ") || "home-proxy"
}

function formatAgentRemoteExtensionSyncSummary(agent: AgentInstance): string {
  if (!agent.remote_execution) {
    return "not applicable"
  }
  const status = agent.remote_extension_manifest_sync
  if (!status) {
    if (!hasActiveHomeProxyExtensionGrants(agent.extension_grants)) {
      return "not applicable (no active home-proxy tools)"
    }
    return `pending, next=${formatAgentRemoteExtensionSyncNextAction(agent)}`
  }
  const details = [
    status.state,
    status.pending_revoke ? "pending revoke" : null,
    status.manifest_hash ? `hash=${status.manifest_hash.slice(0, 12)}` : null,
    status.last_error ? `error=${status.last_error}` : null,
    status.last_synced_at_ms ? `synced=${formatTimestamp(status.last_synced_at_ms)}` : null,
    status.last_attempted_at_ms ? `attempted=${formatTimestamp(status.last_attempted_at_ms)}` : null,
  ].filter(Boolean)
  if (status.state === "failed" || status.state === "stale" || status.pending_revoke || status.last_error) {
    details.push(`next=${formatAgentRemoteExtensionSyncNextAction(agent)}`)
  }
  return details.join(", ")
}

function formatAgentRemoteExtensionSyncNextAction(agent: AgentInstance): string {
  return remoteExtensionSyncNextAction(
    agent.remote_extension_manifest_sync,
    agent.agent_ref,
    agent.remote_execution?.worker_machine_id,
  ) ?? `run /extension sync-status ${agent.agent_ref}`
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
  return slices.find((slice) => slice.agent_ids?.includes(agent.id))
    ?? slices.find((slice) =>
      slice.worker_kernel_id === remote.worker_kernel_id
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
