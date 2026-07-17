import type { RuntimeSession, SliceRecord } from "./kernel-types.js"
import {
  remoteExtensionSyncNextAction,
  workerExtensionSyncNextAction,
} from "./shell-capability-format.js"
import { extensionGrantSource } from "./extension-grant-source.js"
import {
  formatExtensionGrantPlacementSummary,
  shouldShowRemoteExtensionManifestSync,
  shouldShowWorkerExtensionGrantSync,
} from "./extension-grant-placement.js"
import {
  remoteWorkerProviderRunIsMissing,
  remoteWorkerProviderRunRecoveryAction,
} from "./provider-run-recovery.js"
import { formatSessionHomeKernelLabel } from "./session-runtime-labels.js"
import {
  formatSliceProviderAccounts,
  formatSliceProviderAuthReadiness,
  formatSliceProviderList,
  formatSliceScope,
  sliceProviderAuthCoverage,
} from "./slice-format.js"
import { formatWorkspaceLiveSyncModeLabel } from "./workspace-live-sync-mode.js"
import { sessionAgentBusyForProviderRunRecovery, sessionPromptWorkSummary } from "./session-prompt-work.js"
import { sessionAgentRuntimeDisplayState } from "./session-runtime-status.js"

export type SessionRuntimeStatusFormatOptions = {
  readonly slices?: readonly SliceRecord[]
  readonly sliceLookupError?: string | null
}

export function formatSessionRuntimeStatus(
  session: RuntimeSession,
  options: SessionRuntimeStatusFormatOptions = {},
): string {
  const focusedAgent = session.agents.find((agent) => agent.id === session.focused_agent_id) ?? null
  const promptSummary = formatPromptSummary(session)
  const agentSummary = formatAgentSummary(session)
  const remoteSummary = formatRemoteSummary(session, options)
  const homeExtensionSummary = formatExtensionLaneSummary(session, "home")
  const workerExtensionSummary = formatExtensionLaneSummary(session, "worker")
  const nextActions = formatSessionNextActions(session, options)
  const lines = [
    "session runtime",
    `session: ${session.alias ? `${session.alias} (${session.id})` : session.id}`,
    `status: ${session.status}`,
    `home kernel: ${formatSessionHomeKernelLabel(session)}`,
    `session owner: ${session.owner_user_id?.trim() || "-"}`,
    "authority: home owns sessions, prompts, and live sync; each extension source owns its grants, definitions, credentials, and execution",
    `workspace: ${session.workspace_id}`,
    `worktree: ${session.worktree_id}`,
    `live sync: ${formatWorkspaceLiveSyncModeLabel(session.workspace_live_sync_mode)}`,
    "live sync scope: selected workspace/worktree only; other repositories unrestricted",
    `attachments: ${session.attachment_ids.length}`,
    `focused agent: ${focusedAgent ? formatAgentLabel(focusedAgent) : "-"}`,
    `prompts: ${promptSummary}`,
    `agents: ${agentSummary}`,
    `remote runtime: ${remoteSummary}`,
    `home-proxy extensions: ${homeExtensionSummary}`,
    `worker-local extensions: ${workerExtensionSummary}`,
    `provider run: ${session.active_provider_run_id ?? "-"}`,
    "agent runtime:",
    ...formatAgentRuntimeLines(session, options),
  ]
  if (session.collaboration_agent_counts) {
    lines.push(`collaboration: ${formatCollaborationSummary(session)}`)
  }
  for (const action of nextActions) {
    lines.push(`next: ${action}`)
  }
  return lines.join("\n")
}

function formatAgentRuntimeLines(
  session: RuntimeSession,
  options: SessionRuntimeStatusFormatOptions,
): string[] {
  if (session.agents.length === 0) {
    return ["  - none"]
  }
  return session.agents.map((agent) => [
    `  - ${formatAgentLabel(agent)}:`,
    sessionAgentRuntimeDisplayState(session, agent),
    formatAgentProvider(agent),
    `worktree=${agent.worktree_id ?? "-"}`,
    formatAgentPlacement(agent, sliceForRemoteAgent(agent, options.slices ?? []), options.sliceLookupError),
    formatAgentExtensions(agent),
    formatAgentRemoteExtensionSync(agent),
  ].filter(Boolean).join(" "))
}

function formatAgentLabel(agent: RuntimeSession["agents"][number]): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

function formatAgentProvider(agent: RuntimeSession["agents"][number]): string {
  const provider = agent.primary_provider ?? agent.provider
  const model = agent.primary_model ?? agent.model
  if (!model) {
    return provider
  }
  return model.startsWith(`${provider}/`) ? model : `${provider}/${model}`
}

function formatAgentPlacement(
  agent: RuntimeSession["agents"][number],
  slice: SliceRecord | null,
  sliceLookupError: string | null | undefined,
): string {
  const remote = agent.remote_execution
  if (!remote) {
    return "placement=home-local"
  }
  const sliceRef = slice
    ? slice.name || slice.id
    : remote.worker_kernel_id.startsWith("slice:")
    ? remote.worker_kernel_id.slice("slice:".length)
    : null
  const placement = sliceRef ? `slice:${sliceRef}` : "remote"
  const parts = [
    `placement=${placement}`,
    slice ? `slice_status=${slice.status}` : null,
    slice ? `slice_worktree=${formatSliceScope(slice)}` : null,
    slice ? `slice_auth=${formatSliceProviderAuthReadiness(slice)}` : null,
    slice ? `slice_accounts=${formatSliceProviderAccounts(slice)}` : null,
    !slice && sliceLookupError ? `slice_lookup_error=${sliceLookupError}` : null,
    remote.worker_machine_id ? `worker=${remote.worker_machine_id}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `worker_run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return parts.join(" ")
}

function formatAgentExtensions(agent: RuntimeSession["agents"][number]): string {
  const grants = agent.extension_grants ?? []
  if (grants.length === 0) {
    return agent.remote_extension_manifest_sync?.pending_revoke
      || agent.worker_extension_grant_sync?.pending_revoke
      ? "extensions=none(final-revoke-pending)"
      : "extensions=none"
  }
  return `extensions=${formatExtensionGrantPlacementSummary(grants, {
    remote: Boolean(agent.remote_execution),
    countSeparator: "=",
  })}`
}

function formatAgentRemoteExtensionSync(agent: RuntimeSession["agents"][number]): string | null {
  if (!agent.remote_execution) {
    return null
  }
  const homeStatus = agent.remote_extension_manifest_sync
  const workerStatus = agent.worker_extension_grant_sync
  const showHome = shouldShowRemoteExtensionManifestSync(agent.extension_grants, homeStatus)
  const showWorker = shouldShowWorkerExtensionGrantSync(agent.extension_grants, workerStatus)
  if (!showHome && !showWorker) {
    return null
  }
  return [
    showHome ? formatAgentExtensionSyncToken("home_manifest", homeStatus) : null,
    showWorker ? formatAgentExtensionSyncToken("worker_manifest", workerStatus) : null,
  ].filter(Boolean).join(" ")
}

function formatAgentExtensionSyncToken(
  label: "home_manifest" | "worker_manifest",
  status: RuntimeSession["agents"][number]["remote_extension_manifest_sync"],
): string {
  if (!status) {
    return `${label}=pending`
  }
  const details = [
    `${label}=${status.state}`,
    status.manifest_hash ? `hash=${status.manifest_hash.slice(0, 8)}` : null,
    status.pending_revoke ? "pending_revoke=yes" : null,
    status.last_error ? `error=${status.last_error}` : null,
  ].filter(Boolean)
  return details.join(" ")
}

function sliceForRemoteAgent(
  agent: RuntimeSession["agents"][number],
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
      || slice.worker_machine_id === remote.worker_machine_id
    ) ?? null
}

function formatPromptSummary(session: RuntimeSession): string {
  const summary = sessionPromptWorkSummary(session)
  return `active=${summary.active}, queued=${summary.queued}, busy_agents=${summary.busyAgents}`
}

function formatAgentSummary(session: RuntimeSession): string {
  const total = session.agents.length
  const local = session.agents.filter((agent) => !agent.remote_execution).length
  const remote = total - local
  const slices = session.agents.filter((agent) => agent.remote_execution?.worker_kernel_id?.startsWith("slice:")).length
  return `${total} total, ${local} local, ${remote} remote/slice${slices ? `, ${slices} slice` : ""}`
}

function formatRemoteSummary(
  session: RuntimeSession,
  options: SessionRuntimeStatusFormatOptions,
): string {
  const remoteAgents = session.agents.filter((agent) => agent.remote_execution)
  if (remoteAgents.length === 0) {
    return "none"
  }
  const slices = options.slices ?? []
  const sliceRefs = new Set(remoteAgents.map((agent) => {
    const slice = sliceForRemoteAgent(agent, slices)
    if (slice) {
      return slice.name || slice.id
    }
    const workerKernelId = agent.remote_execution?.worker_kernel_id ?? ""
    return workerKernelId.startsWith("slice:") ? workerKernelId.slice("slice:".length) : null
  }).filter(Boolean))
  const workerRunGaps = remoteAgents.filter((agent) => {
    if (agent.remote_execution?.active_worker_provider_run_id) return false
    return remoteAgentHasWorkerRunGap(session, agent)
  })
  const workers = new Set(remoteAgents.map((agent) => (
    agent.remote_execution?.worker_machine_id
      || agent.remote_execution?.worker_kernel_id
      || "-"
  )))
  const parts = [
    `${remoteAgents.length} agent${remoteAgents.length === 1 ? "" : "s"}`,
    `${workers.size} worker${workers.size === 1 ? "" : "s"}`,
    sliceRefs.size > 0 ? `${sliceRefs.size} slice${sliceRefs.size === 1 ? "" : "s"}` : null,
    `${workerRunGaps.length} worker run gap${workerRunGaps.length === 1 ? "" : "s"}`,
  ].filter(Boolean)
  return parts.join(", ")
}

function formatExtensionLaneSummary(
  session: RuntimeSession,
  source: "home" | "worker",
): string {
  const laneGrants = session.agents.flatMap((agent) => (agent.extension_grants ?? []).filter((grant) => (
    extensionGrantSource(grant) === source
    && (source === "worker" || grant.kind !== "skill")
  )))
  const laneAgents = session.agents.filter((agent) => (
    agent.remote_execution
    && (
      (agent.extension_grants ?? []).some((grant) => (
        extensionGrantSource(grant) === source
        && (source === "worker" || grant.kind !== "skill")
      ))
      || (source === "worker"
        ? agent.worker_extension_grant_sync?.pending_revoke
        : agent.remote_extension_manifest_sync?.pending_revoke)
    )
  ))
  const syncIssues = session.agents.filter((agent) => {
    const sync = source === "worker"
      ? agent.worker_extension_grant_sync
      : agent.remote_extension_manifest_sync
    return Boolean(sync && (sync.state !== "synced" || sync.pending_revoke || sync.last_error))
  })
  const pendingRevokes = syncIssues.filter((agent) => (
    source === "worker"
      ? agent.worker_extension_grant_sync?.pending_revoke
      : agent.remote_extension_manifest_sync?.pending_revoke
  ))
  if (laneAgents.length === 0 && syncIssues.length === 0) {
    return "none"
  }
  const kindCounts = ["mcp", "script", "connector", "skill"]
    .map((kind) => {
      const count = laneGrants.filter((grant) => grant.kind === kind).length
      return count > 0 ? `${kind}=${count}` : null
    })
    .filter(Boolean)
    .join(", ")
  const grantSummary = `${laneGrants.length} grant${laneGrants.length === 1 ? "" : "s"}${kindCounts ? ` (${kindCounts})` : ""}`
  return `${laneAgents.length} agent${laneAgents.length === 1 ? "" : "s"}, ${grantSummary}, ${syncIssues.length} sync issue${syncIssues.length === 1 ? "" : "s"}, ${pendingRevokes.length} pending revoke${pendingRevokes.length === 1 ? "" : "s"}`
}

function formatCollaborationSummary(session: RuntimeSession): string {
  const counts = session.collaboration_agent_counts
  if (!counts) {
    return "none"
  }
  return `${counts.collaborator_count} collaborator${counts.collaborator_count === 1 ? "" : "s"}, ${counts.owned_agent_count} mine, ${counts.other_user_agent_count} others, ${counts.total_agent_count} total`
}

function formatSessionNextActions(
  session: RuntimeSession,
  options: SessionRuntimeStatusFormatOptions,
): string[] {
  const actions: string[] = []
  const workerGapAgent = session.agents.find((agent) => (
    agent.remote_execution
    && !agent.remote_execution.active_worker_provider_run_id
    && remoteAgentHasWorkerRunGap(session, agent)
  ))
  if (workerGapAgent) {
    actions.push(remoteWorkerProviderRunRecoveryAction(
      workerGapAgent.agent_ref,
      workerGapAgent.remote_execution?.worker_machine_id,
    ))
  }
  const homeExtensionIssueAgent = session.agents.find((agent) => {
    const sync = agent.remote_extension_manifest_sync
    return Boolean(sync && (sync.state !== "synced" || sync.pending_revoke || sync.last_error))
  })
  if (homeExtensionIssueAgent) {
    const next = remoteExtensionSyncNextAction(
      homeExtensionIssueAgent.remote_extension_manifest_sync,
      homeExtensionIssueAgent.agent_ref,
      homeExtensionIssueAgent.remote_execution?.worker_machine_id,
    )
    if (next) {
      actions.push(next)
    }
  }
  const workerExtensionIssueAgent = session.agents.find((agent) => {
    const sync = agent.worker_extension_grant_sync
    return Boolean(sync && (sync.state !== "synced" || sync.pending_revoke || sync.last_error))
  })
  if (workerExtensionIssueAgent) {
    const next = workerExtensionSyncNextAction(
      workerExtensionIssueAgent.worker_extension_grant_sync,
      workerExtensionIssueAgent.agent_ref,
      workerExtensionIssueAgent.remote_execution?.worker_machine_id,
    )
    if (next) {
      actions.push(next)
    }
  }
  const sliceAuthAction = formatSliceAuthNextAction(session, options.slices ?? [])
  if (sliceAuthAction) {
    actions.push(sliceAuthAction)
  }
  return actions
}

function remoteAgentHasWorkerRunGap(session: RuntimeSession, agent: RuntimeSession["agents"][number]): boolean {
  return remoteWorkerProviderRunIsMissing({
    agent,
    agentBusy: sessionAgentBusyForProviderRunRecovery(session, agent.id),
  })
}

function formatSliceAuthNextAction(
  session: RuntimeSession,
  slices: readonly SliceRecord[],
): string | null {
  for (const agent of session.agents) {
    const slice = sliceForRemoteAgent(agent, slices)
    if (!slice) {
      continue
    }
    const coverage = sliceProviderAuthCoverage(slice)
    const ref = slice.name || slice.id
    if (coverage.missingProviders.length > 0) {
      const provider = coverage.missingProviders[0]!
      const list = formatSliceProviderList(coverage.missingProviders)
      return `run /slice doctor ${ref}; configure missing provider account${coverage.missingProviders.length === 1 ? "" : "s"} ${list} with /slice auth import ${ref} ${provider} or /slice auth login ${ref} ${provider} before sending prompts to agents in that slice`
    }
    if (coverage.staleProviders.length > 0) {
      const provider = coverage.staleProviders[0]!
      const list = formatSliceProviderList(coverage.staleProviders)
      return `run /slice doctor ${ref}; refresh provider account${coverage.staleProviders.length === 1 ? "" : "s"} ${list} with /slice auth login ${ref} ${provider} before sending prompts to agents in that slice`
    }
  }
  return null
}
