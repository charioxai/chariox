import type { RuntimeSession } from "./kernel-types.js"
import { remoteWorkerProviderRunRecoveryAction } from "./provider-run-recovery.js"
import { formatWorkspaceLiveSyncModeLabel } from "./workspace-live-sync-mode.js"

export function formatSessionRuntimeStatus(session: RuntimeSession): string {
  const focusedAgent = session.agents.find((agent) => agent.id === session.focused_agent_id) ?? null
  const promptSummary = formatPromptSummary(session)
  const agentSummary = formatAgentSummary(session)
  const remoteSummary = formatRemoteSummary(session)
  const extensionSummary = formatRemoteExtensionSummary(session)
  const nextActions = formatSessionNextActions(session)
  const lines = [
    "session runtime",
    `session: ${session.alias ? `${session.alias} (${session.id})` : session.id}`,
    `status: ${session.status}`,
    `home kernel: ${formatSessionHomeKernel(session)}`,
    `session owner: ${session.owner_user_id?.trim() || "-"}`,
    "authority: home owns sessions, prompts, grants, and live sync; workers execute leases and projected tools",
    `workspace: ${session.workspace_id}`,
    `worktree: ${session.worktree_id}`,
    `live sync: ${formatWorkspaceLiveSyncModeLabel(session.workspace_live_sync_mode)}`,
    "live sync scope: selected workspace/worktree only; other repositories unrestricted",
    `attachments: ${session.attachment_ids.length}`,
    `focused agent: ${focusedAgent ? formatAgentLabel(focusedAgent) : "-"}`,
    `prompts: ${promptSummary}`,
    `agents: ${agentSummary}`,
    `remote runtime: ${remoteSummary}`,
    `home-proxy extensions: ${extensionSummary}`,
    `provider run: ${session.active_provider_run_id ?? "-"}`,
  ]
  if (session.collaboration_agent_counts) {
    lines.push(`collaboration: ${formatCollaborationSummary(session)}`)
  }
  for (const action of nextActions) {
    lines.push(`next: ${action}`)
  }
  return lines.join("\n")
}

function formatSessionHomeKernel(session: RuntimeSession): string {
  const kernel = session.host_daemon_id?.trim() || ""
  const machine = session.host_machine_id?.trim() || ""
  if (kernel && machine) {
    return `${kernel}@${machine}`
  }
  return kernel || machine || "-"
}

function formatAgentLabel(agent: RuntimeSession["agents"][number]): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

function formatPromptSummary(session: RuntimeSession): string {
  const active = session.active_prompt ? "active=1" : "active=0"
  const queued = `queued=${session.queued_prompts.length}`
  const busyAgents = session.agents.filter((agent) => agent.is_processing || agent.state === "Working").length
  return `${active}, ${queued}, busy_agents=${busyAgents}`
}

function formatAgentSummary(session: RuntimeSession): string {
  const total = session.agents.length
  const local = session.agents.filter((agent) => !agent.remote_execution).length
  const remote = total - local
  const slices = session.agents.filter((agent) => agent.remote_execution?.worker_kernel_id?.startsWith("slice:")).length
  return `${total} total, ${local} local, ${remote} remote/slice${slices ? `, ${slices} slice` : ""}`
}

function formatRemoteSummary(session: RuntimeSession): string {
  const remoteAgents = session.agents.filter((agent) => agent.remote_execution)
  if (remoteAgents.length === 0) {
    return "none"
  }
  const workerRunGaps = remoteAgents.filter((agent) => {
    if (agent.remote_execution?.active_worker_provider_run_id) return false
    return agent.state === "Working" || agent.is_processing
  })
  const workers = new Set(remoteAgents.map((agent) => (
    agent.remote_execution?.worker_machine_id
      || agent.remote_execution?.worker_kernel_id
      || "-"
  )))
  return `${remoteAgents.length} agent${remoteAgents.length === 1 ? "" : "s"}, ${workers.size} worker${workers.size === 1 ? "" : "s"}, ${workerRunGaps.length} worker run gap${workerRunGaps.length === 1 ? "" : "s"}`
}

function formatRemoteExtensionSummary(session: RuntimeSession): string {
  const homeProxyAgents = session.agents.filter((agent) => (
    agent.remote_execution
    && (agent.extension_grants ?? []).some((grant) => grant.kind !== "skill")
  ))
  const syncIssues = session.agents.filter((agent) => {
    const sync = agent.remote_extension_manifest_sync
    return Boolean(sync && (sync.state !== "synced" || sync.pending_revoke || sync.last_error))
  })
  const pendingRevokes = syncIssues.filter((agent) => agent.remote_extension_manifest_sync?.pending_revoke)
  if (homeProxyAgents.length === 0 && syncIssues.length === 0) {
    return "none"
  }
  return `${homeProxyAgents.length} agent${homeProxyAgents.length === 1 ? "" : "s"}, ${syncIssues.length} sync issue${syncIssues.length === 1 ? "" : "s"}, ${pendingRevokes.length} pending revoke${pendingRevokes.length === 1 ? "" : "s"}`
}

function formatCollaborationSummary(session: RuntimeSession): string {
  const counts = session.collaboration_agent_counts
  if (!counts) {
    return "none"
  }
  return `${counts.collaborator_count} collaborator${counts.collaborator_count === 1 ? "" : "s"}, ${counts.owned_agent_count} mine, ${counts.other_user_agent_count} others, ${counts.total_agent_count} total`
}

function formatSessionNextActions(session: RuntimeSession): string[] {
  const actions: string[] = []
  const workerGapAgent = session.agents.find((agent) => (
    agent.remote_execution
    && !agent.remote_execution.active_worker_provider_run_id
    && (agent.state === "Working" || agent.is_processing)
  ))
  if (workerGapAgent) {
    actions.push(remoteWorkerProviderRunRecoveryAction(
      workerGapAgent.agent_ref,
      workerGapAgent.remote_execution?.worker_machine_id,
    ))
  }
  const extensionIssueAgent = session.agents.find((agent) => {
    const sync = agent.remote_extension_manifest_sync
    return Boolean(sync && (sync.state !== "synced" || sync.pending_revoke || sync.last_error))
  })
  if (extensionIssueAgent) {
    actions.push(`run /extension sync-status ${extensionIssueAgent.agent_ref}; use /extension sync-retry ${extensionIssueAgent.agent_ref} after worker connectivity is healthy`)
  }
  return actions
}
