import type { AgentInstance, SliceRecord } from "./kernel-types.js"
import { formatRemoteExtensionSyncStatusLine, remoteExtensionSyncNextAction } from "./shell-capability-format.js"
import {
  formatExtensionAuthorityBoundaryDetail,
  formatExtensionGrantRuntimeDetail,
  formatExtensionGrantPlacement,
  formatExtensionGrantPlacementSummary,
  hasActiveHomeProxyExtensionGrants,
  shouldShowRemoteExtensionManifestSync,
} from "./extension-grant-placement.js"
import {
  formatSliceProviderAccounts,
  formatSliceProviderAuthReadiness,
  formatSliceProviderAuthStatus,
  formatSliceScope,
} from "./slice-format.js"
import { formatWorkspaceLiveSyncModeLabel } from "./workspace-live-sync-mode.js"
import { providerRunRecoveryActions } from "./provider-run-recovery.js"

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
  workspaceLiveSyncWorktree?: string | null
}

export function formatAgentRef(agent: AgentInstance): string {
  const meta = Boolean(agent.meta_mode)
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}${meta ? " [Meta mode]" : ""}`
}

export function formatAgentListSummary(
  agents: AgentInstance[],
  slices: readonly SliceRecord[] = [],
  providerRunContext: ShellAgentProviderRunContext = {},
  sessionContext: ShellAgentSessionContext = {},
): string {
  const prefix = formatAgentListSessionContext(sessionContext)
  if (agents.length === 0) {
    return prefix ? `${prefix}\nno agents in session` : "no agents in session"
  }
  const agentList = agents
    .map((agent) => formatAgentListEntry(agent, sliceForRemoteAgent(agent, slices), providerRunContext))
    .join(", ")
  const summary = `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
  return prefix ? `${prefix}\n${summary}` : summary
}

function formatAgentListSessionContext(context: ShellAgentSessionContext): string | null {
  if (!context.homeKernelId && !context.homeMachineId && !context.ownerUserId && !context.workspaceLiveSyncMode) {
    return null
  }
  const parts = [
    `home kernel ${formatHomeKernel(context)}`,
    context.ownerUserId ? `owner ${context.ownerUserId}` : null,
    "authority home-owned",
    `live sync ${formatWorkspaceLiveSyncModeLabel(context.workspaceLiveSyncMode)}${formatLiveSyncScopeSuffix(context)}`,
  ].filter(Boolean)
  return `session runtime: ${parts.join("; ")}`
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
    formatAgentListSliceAuth(slice),
    formatAgentListProviderRun(agent, providerRunContext),
    formatAgentListProviderRunHealth(agent),
    agent.execution_mode_override ? `mode ${agent.execution_mode_override}` : null,
    agent.permission_level_override ? `permissions ${agent.permission_level_override}` : null,
    formatAgentListGrantCount(agent),
    formatAgentListRemoteExtensionSync(agent),
  ].filter(Boolean)
  const meta = Boolean(agent.meta_mode)
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}${meta ? " [Meta mode]" : ""} [${parts.join("; ")}]`
}

function formatAgentListSliceAuth(slice: SliceRecord | null): string | null {
  if (!slice) {
    return null
  }
  return formatSliceProviderAuthStatus(slice)
}

function formatAgentListProviderRun(
  agent: AgentInstance,
  context: ShellAgentProviderRunContext,
): string | null {
  const runId = agentProviderRunId(agent, context)
  return runId ? `session run ${runId}` : null
}

function formatAgentListProviderRunHealth(agent: AgentInstance): string | null {
  const remote = agent.remote_execution
  if (!remote || remote.active_worker_provider_run_id) {
    return null
  }
  if (agent.state !== "Working" && !agent.is_processing) {
    return null
  }
  const worker = remote.worker_machine_id ? ` on ${remote.worker_machine_id}` : ""
  return `provider blocked (missing worker run${worker}; inspect ${agent.agent_ref})`
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
  const details = formatAgentListRemotePlacementDetails(remote)
  if (slice) {
    return `slice ${slice.name || slice.id}${details}`
  }
  const machine = remote.worker_machine_id ? `@${remote.worker_machine_id}` : ""
  return `remote ${remote.worker_kernel_id}${machine}${details}`
}

function formatAgentListRemotePlacementDetails(remote: NonNullable<AgentInstance["remote_execution"]>): string {
  const details = [
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return details.length ? ` (${details.join(", ")})` : ""
}

function formatAgentListGrantCount(agent: AgentInstance): string {
  const grants = agent.extension_grants?.length ?? 0
  const grantList = agent.extension_grants ?? []
  const count = `${grantList.length} grant${grantList.length === 1 ? "" : "s"}`
  if (grants === 0 && agent.remote_extension_manifest_sync?.pending_revoke) {
    return `${count} (final revoke pending)`
  }
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
    return `manifest pending; see /extension sync-status ${agent.agent_ref}`
  }
  const hash = status.manifest_hash ? ` ${status.manifest_hash.slice(0, 8)}` : ""
  const revoke = status.pending_revoke ? " pending revoke" : ""
  const error = status.last_error ? ` error ${status.last_error}` : ""
  const action = status.state !== "synced" || status.pending_revoke || status.last_error
    ? `; see /extension sync-status ${agent.agent_ref}`
    : ""
  return `manifest ${status.state}${hash}${revoke}${error}${action}`
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
    "runtime authority: home owns session, prompts, grants, and live sync; workers execute leases and projected tools",
    `live sync: ${formatWorkspaceLiveSyncModeLabel(sessionContext.workspaceLiveSyncMode)}`,
    `live sync scope: ${formatLiveSyncScope(sessionContext)}`,
    `provider: ${agent.provider}`,
    `model: ${agent.model ?? "<none>"}`,
    `variant: ${agent.effort ?? "<none>"}`,
    `mode: ${agent.execution_mode_override ?? "session"}`,
    `permissions: ${agent.permission_level_override ?? "session"}`,
    `workspace: ${agent.workspace_id ?? "<none>"}`,
    `worktree: ${agent.worktree_id ?? "<none>"}`,
    `placement: ${formatAgentPlacement(agent, slice)}`,
    `provider run: ${formatAgentProviderRunSummary(agent, providerRunContext)}`,
    ...formatAgentProviderRunNextAction(agent, providerRunContext),
    ...(slice ? [
      `slice: ${formatSliceSummary(slice)}`,
      `slice provider auth: ${formatSliceProviderAuthReadiness(slice)}`,
      `slice provider accounts: ${formatSliceProviderAccounts(slice)}`,
    ] : sliceLookupError ? [
      `slice lookup: ${sliceLookupError}`,
    ] : []),
    `extensions: ${formatAgentExtensionSummary(agent)}`,
    `extension runtime: ${formatExtensionGrantRuntimeDetail(agent.extension_grants, Boolean(agent.remote_execution))}`,
    `extension boundary: ${formatExtensionAuthorityBoundaryDetail(agent.extension_grants, Boolean(agent.remote_execution))}`,
    ...formatAgentRemoteExtensionSyncLines(agent),
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

function formatLiveSyncScope(context: ShellAgentSessionContext): string {
  const worktree = context.workspaceLiveSyncWorktree?.trim()
  return worktree
    ? `${worktree} (selected workspace/worktree only; other repositories unrestricted)`
    : "selected workspace/worktree only; other repositories unrestricted"
}

function formatLiveSyncScopeSuffix(context: ShellAgentSessionContext): string {
  const worktree = context.workspaceLiveSyncWorktree?.trim()
  return worktree ? ` on ${worktree}` : ""
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
  const footer = agent.last_substitution?.reason
    ? `\nlast substitution: ${agent.last_substitution.reason}`
    : ""
  return `${formatAgentRef(agent)} substitutes (${substitutes.length}, timeout ${timeout}):\n${lines.join("\n")}${footer}`
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

function formatAgentProviderRunNextAction(
  agent: AgentInstance,
  context: ShellAgentProviderRunContext,
): string[] {
  return providerRunRecoveryActions({
    agent,
    activeProviderRunId: context.activeProviderRunId,
    activeProviderRunAgentId: context.activeProviderRunAgentId,
  }).map((action) => `provider run next: ${action}`)
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
    return agent.remote_extension_manifest_sync?.pending_revoke ? "none (final revoke pending)" : "none"
  }
  return formatExtensionGrantPlacementSummary(grants, {
    remote: Boolean(agent.remote_execution),
    countSeparator: "=",
  })
}

export function formatAgentExtensionPlacementSummary(agent: AgentInstance): string {
  return formatExtensionGrantPlacement(agent.extension_grants, Boolean(agent.remote_execution))
}

function formatAgentRemoteExtensionSyncLines(agent: AgentInstance): string[] {
  if (!agent.remote_execution) {
    return ["remote extension sync: not applicable (worker-local agent; no home-proxy manifest)"]
  }
  const status = agent.remote_extension_manifest_sync
  if (!status) {
    if (!hasActiveHomeProxyExtensionGrants(agent.extension_grants)) {
      return ["remote extension sync: not applicable (no active home-proxy tools)"]
    }
    return [
      `remote extension sync: ${formatRemoteExtensionSyncStatusLine(null, {
        includeNext: false,
        agentRef: agent.agent_ref,
        workerMachineId: agent.remote_execution.worker_machine_id,
        errorPrefix: "error=",
      })}`,
      `remote extension next: ${formatAgentRemoteExtensionSyncNextAction(agent)}`,
    ]
  }
  const details = [
    formatRemoteExtensionSyncStatusLine(status, {
      includeHash: true,
      includeNext: false,
      errorPrefix: "error=",
    }),
    status.last_synced_at_ms ? `synced=${formatTimestamp(status.last_synced_at_ms)}` : null,
    status.last_attempted_at_ms ? `attempted=${formatTimestamp(status.last_attempted_at_ms)}` : null,
  ].filter(Boolean)
  const lines = [`remote extension sync: ${details.join(", ")}`]
  if (status.state !== "synced" || status.pending_revoke || status.last_error) {
    lines.push(`remote extension next: ${formatAgentRemoteExtensionSyncNextAction(agent)}`)
  }
  return lines
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
  const worktree = formatSliceScope(slice)
  return `${slice.name || slice.id} (${[
    `id=${slice.id}`,
    `status=${slice.status}`,
    `owner=${formatSliceOwner(slice)}`,
    "authority=home-managed",
    `display=${slice.display_mode ?? "headless"}`,
    `worktree=${worktree}`,
    `agents=${slice.agent_ids?.length ?? 0}`,
  ].join(", ")})`
}

function formatSliceOwner(slice: SliceRecord): string {
  const kernel = slice.owner_kernel_id?.trim() || ""
  const machine = slice.owner_machine_id?.trim() || ""
  if (kernel && machine) {
    return `${kernel}@${machine}`
  }
  return kernel || machine || "unknown"
}
