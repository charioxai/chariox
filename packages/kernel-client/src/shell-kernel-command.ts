import type { DaemonHealthProjection, RuntimeSession } from "./kernel-types.js"
import { deleteKernelRequest, getDaemonHealthRequest } from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellKernelCommandDeps = {
  client: ShellKernelClient
}

export async function executeKernelCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellKernelCommandDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  if ((action === "health" || action === "status") && args.length === 0) {
    const response = await deps.client.send(getDaemonHealthRequest())
    const payload = expectVariant<{ projection: DaemonHealthProjection }>(response, "DaemonHealth")
    const issueCount = kernelHealthIssueCount(payload.projection)
    return {
      ok: issueCount === 0,
      message: formatKernelHealth(payload.projection, issueCount),
      data: payload.projection,
    }
  }
  if (action !== "delete" || args.length > 0) {
    return { ok: false, message: "usage: kernel health|status|delete" }
  }
  const response = await deps.client.send(deleteKernelRequest())
  const payload = expectVariant<{ kernel_id: string; deleted_sessions: RuntimeSession[] }>(response, "KernelDeleted")
  const deletedCurrentSession = context.sessionId
    ? payload.deleted_sessions.some((session) => session.id === context.sessionId)
    : false
  return {
    ok: true,
    message: `deleted kernel ${payload.kernel_id} (${payload.deleted_sessions.length} session${payload.deleted_sessions.length === 1 ? "" : "s"})`,
    contextUpdates: deletedCurrentSession
      ? { sessionId: undefined, attachmentId: undefined, agentId: undefined }
      : undefined,
    data: payload,
  }
}

export function kernelHealthIssueCount(health: DaemonHealthProjection): number {
  return health.provider_runs.duplicate_arroba_agent_bindings.length
    + health.provider_runs.multi_interface_agent_bindings.length
    + health.provider_runs.orphaned_active_runs.length
    + health.provider_runs.session_active_run_mismatches.length
    + (health.provider_catalog.expired ? 1 : 0)
    + health.projection_invariants.mismatches.length
    + health.workspace_coordination.worktree_collisions.length
    + health.workspace_live_sync.workspace_identity.identity_changed_provider_runs
    + health.workspace_live_sync.workspace_identity.invalid_provider_runs
    + health.workspace_live_sync.external_changes.externally_changed_artifacts
    + health.workspace_live_sync.external_changes.live_watcher_scan_errors
    + (health.workspace_live_sync.managed_mode.write_fence_supported ? 0 : (health.workspace_live_sync.managed_mode.unavailable_reason ? 1 : 0))
    + (health.slice_lifecycle.issues.length > 0
      ? health.slice_lifecycle.issues.length
      : health.slice_lifecycle.unhealthy_slices + health.slice_lifecycle.failed_operations)
    + (health.slice_lifecycle.provider_auth_issues.length > 0
      ? health.slice_lifecycle.provider_auth_issues.length
      : health.slice_lifecycle.provider_auth_missing_slices + health.slice_lifecycle.provider_auth_unconfigured_slices)
    + (health.remote_execution.issues.length > 0
      ? health.remote_execution.issues.length
      : health.remote_execution.missing_active_worker_runs + health.remote_execution.malformed_bindings)
    + (health.remote_extension_sync.issues.length > 0
      ? health.remote_extension_sync.issues.length
      : health.remote_extension_sync.failed_agents
        + health.remote_extension_sync.stale_agents
        + health.remote_extension_sync.manifest_missing_agents
        + health.remote_extension_sync.pending_revoke_agents)
    + health.transport.replay_gaps
    + health.transport.inbound_overload_rejections
    + health.transport.duplicate_command_conflicts
    + health.transport.outgoing_queue_overflows
    + health.transport.slow_consumer_closes
    + health.terminal_stream.trimmed_pending_output_recipients
    + health.capability_executor.rejected_jobs
    + health.capability_executor.join_errors
    + health.provider_run_actor.enqueue_rejections
}

function formatKernelHealth(health: DaemonHealthProjection, issueCount: number): string {
  const lines = [
    `kernel health: ${issueCount === 0 ? "ok" : `${issueCount} issue${issueCount === 1 ? "" : "s"}`}`,
    `provider runs: projected=${health.provider_runs.projected_runs} active=${health.provider_runs.active_runs} arroba=${health.provider_runs.arroba_active_runs} native_tui=${health.provider_runs.native_tui_active_runs}`,
    `remote execution: agents=${health.remote_execution.remote_agents} active=${health.remote_execution.active_remote_agents} missing_worker_runs=${health.remote_execution.missing_active_worker_runs} malformed=${health.remote_execution.malformed_bindings}`,
    `remote extensions: home_proxy_agents=${health.remote_extension_sync.home_proxy_agents} grants=${health.remote_extension_sync.home_proxy_grants} synced=${health.remote_extension_sync.synced_agents} failed=${health.remote_extension_sync.failed_agents} stale=${health.remote_extension_sync.stale_agents} missing=${health.remote_extension_sync.manifest_missing_agents} pending_revoke=${health.remote_extension_sync.pending_revoke_agents}`,
    `slices: total=${health.slice_lifecycle.total_slices} running=${health.slice_lifecycle.running_slices} unhealthy=${health.slice_lifecycle.unhealthy_slices} auth_missing=${health.slice_lifecycle.provider_auth_missing_slices} auth_unconfigured=${health.slice_lifecycle.provider_auth_unconfigured_slices}`,
    `workspace live sync: reservations=${health.workspace_live_sync.active_reservations} tracked_runs=${health.workspace_live_sync.workspace_identity.tracked_provider_runs} external_changes=${health.workspace_live_sync.external_changes.externally_changed_artifacts} watcher_errors=${health.workspace_live_sync.external_changes.live_watcher_scan_errors} managed_fence=${health.workspace_live_sync.managed_mode.write_fence_supported ? "yes" : "no"}`,
    `transport: connections=${health.transport.active_connections} subscriptions=${health.transport.active_subscriptions} replay_gaps=${health.transport.replay_gaps} overloads=${health.transport.inbound_overload_rejections}`,
  ]
  appendProviderRunIssues(lines, health)
  appendRemoteExecutionIssues(lines, health)
  appendRemoteExtensionIssues(lines, health)
  appendSliceIssues(lines, health)
  appendWorkspaceLiveSyncIssues(lines, health)
  if (health.projection_invariants.mismatches.length > 0) {
    lines.push(`projection invariants: ${health.projection_invariants.mismatches.length} mismatch${health.projection_invariants.mismatches.length === 1 ? "" : "es"}`)
  }
  return lines.join("\n")
}

function appendProviderRunIssues(lines: string[], health: DaemonHealthProjection): void {
  for (const conflict of health.provider_runs.duplicate_arroba_agent_bindings) {
    lines.push(`duplicate provider binding: session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
  }
  for (const conflict of health.provider_runs.multi_interface_agent_bindings) {
    lines.push(`multi-interface provider binding: session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
  }
  for (const issue of health.provider_runs.orphaned_active_runs) {
    lines.push(`orphaned provider run: run=${issue.provider_run_id} session=${issue.session_id} agent=${issue.agent_id ?? "-"} ${issue.details}`)
  }
  for (const issue of health.provider_runs.session_active_run_mismatches) {
    lines.push(`provider run pointer issue: session=${issue.session_id} active=${issue.active_provider_run_id ?? "-"} ${issue.details}`)
  }
}

function appendRemoteExecutionIssues(lines: string[], health: DaemonHealthProjection): void {
  for (const issue of health.remote_execution.issues) {
    lines.push(`remote execution issue: agent=${issue.agent_ref} worker=${issue.worker_kernel_id}/${issue.worker_machine_id} lease=${issue.execution_lease_id} state=${issue.state} kind=${issue.kind}${issue.details ? ` ${issue.details}` : ""}`)
  }
}

function appendRemoteExtensionIssues(lines: string[], health: DaemonHealthProjection): void {
  for (const issue of health.remote_extension_sync.issues) {
    lines.push(`remote extension issue: agent=${issue.agent_ref} worker=${issue.worker_kernel_id}/${issue.worker_machine_id} state=${issue.state}${issue.pending_revoke ? " pending_revoke=yes" : ""}${issue.last_error ? ` ${issue.last_error}` : ""}`)
  }
}

function appendSliceIssues(lines: string[], health: DaemonHealthProjection): void {
  for (const issue of health.slice_lifecycle.issues) {
    lines.push(`slice issue: ${issue.name} (${issue.slice_id}) status=${issue.status}${issue.last_error ? ` ${issue.last_error}` : ""}`)
  }
  for (const issue of health.slice_lifecycle.provider_auth_issues) {
    lines.push(`slice auth issue: ${issue.name} (${issue.slice_id}) provider=${issue.provider ?? "-"} state=${issue.provider_auth_state ?? "-"} ${issue.details}`)
  }
}

function appendWorkspaceLiveSyncIssues(lines: string[], health: DaemonHealthProjection): void {
  const managed = health.workspace_live_sync.managed_mode
  if (!managed.write_fence_supported && managed.unavailable_reason) {
    lines.push(`workspace live sync managed unavailable: ${managed.unavailable_reason}`)
  }
  for (const issue of health.workspace_live_sync.workspace_identity.issues) {
    lines.push(`workspace identity issue: run=${issue.provider_run_id} root=${issue.root} branch=${issue.baseline_branch ?? "-"}->${issue.current_branch ?? "-"} head=${issue.baseline_head_commit ?? "-"}->${issue.current_head_commit ?? "-"}`)
  }
  for (const issue of health.workspace_live_sync.external_changes.issues) {
    lines.push(`workspace external change: run=${issue.provider_run_id ?? "-"} root=${issue.workspace_root ?? "-"} path=${issue.path}`)
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
