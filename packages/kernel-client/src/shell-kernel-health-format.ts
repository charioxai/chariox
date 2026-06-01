import type { DaemonHealthProjection } from "./kernel-types.js"

export function kernelHealthIssueCount(health: DaemonHealthProjection): number {
  return health.provider_runs.duplicate_arroba_agent_bindings.length
    + health.provider_runs.multi_interface_agent_bindings.length
    + health.provider_runs.orphaned_active_runs.length
    + health.provider_runs.session_active_run_mismatches.length
    + providerCatalogHealthIssueCount(health)
    + health.projection_invariants.mismatches.length
    + workspaceHealthIssueCount(health)
    + sliceLifecycleIssueCount(health)
    + remoteExecutionIssueCount(health)
    + remoteExtensionSyncIssueCount(health)
    + transportHealthIssueCount(health)
    + terminalStreamHealthIssueCount(health)
    + capabilityHealthIssueCount(health)
    + providerRunActorHealthIssueCount(health)
    + commandLaneHealthIssueCount(health)
}

export function formatKernelHealth(health: DaemonHealthProjection): string {
  const providerRuns = health.provider_runs
  const providerCatalog = health.provider_catalog
  const providerRunActor = health.provider_run_actor
  const process = health.process
  const capability = health.capability_executor
  const transport = health.transport
  const terminalStream = health.terminal_stream
  const sliceLifecycle = health.slice_lifecycle
  const remoteExecution = health.remote_execution
  const remoteExtensionSync = health.remote_extension_sync
  const workspaceCoordination = health.workspace_coordination
  const liveSync = health.workspace_live_sync
  const liveSyncManagedMode = liveSync.managed_mode
  const workspaceIdentity = liveSync.workspace_identity
  const externalChanges = liveSync.external_changes
  const commandLanes = commandLaneHealthSummary(health)
  const lines = [
    "kernel health",
    `command lanes: session=${commandLanes.session.lanes}/${commandLanes.session.queued} agent=${commandLanes.agent.lanes}/${commandLanes.agent.queued} workflow=${commandLanes.workflow.lanes}/${commandLanes.workflow.queued} provider=${commandLanes.provider.lanes}/${commandLanes.provider.queued} saturated=${commandLanes.saturated}`,
    `process: pid=${process.process_id} rss=${formatBytes(process.current_resident_set_bytes ?? null)} peak_rss=${formatBytes(process.peak_resident_set_bytes ?? null)}`,
    `provider catalog: cached=${providerCatalog.cached ? "yes" : "no"} expired=${providerCatalog.expired ? "yes" : "no"} age=${formatDuration(providerCatalog.age_ms ?? null)} ttl=${formatDuration(providerCatalog.ttl_ms)}`,
    `provider runs: projected=${providerRuns.projected_runs} active=${providerRuns.active_runs} arroba=${providerRuns.arroba_active_runs} native_tui=${providerRuns.native_tui_active_runs}`,
    `provider run actor: enqueued=${providerRunActor.enqueued_commands} rejected=${providerRunActor.enqueue_rejections}`,
    `capabilities: running=${capability.running_jobs}/${capability.max_concurrent_jobs} submitted=${capability.submitted_jobs} failed=${capability.failed_jobs} rejected=${capability.rejected_jobs} join_errors=${capability.join_errors}`,
    `transport: connections=${transport.active_connections} subscriptions=${transport.active_subscriptions} incoming=${transport.incoming_requests} emitted=${transport.emitted_events} replay_gaps=${transport.replay_gaps} overloads=${transport.inbound_overload_rejections} duplicate_commands=${transport.duplicate_command_conflicts} outgoing_overflows=${transport.outgoing_queue_overflows} slow_consumers=${transport.slow_consumer_closes}`,
    `terminal stream: pending_output=${terminalStream.pending_output_records} pending_notices=${terminalStream.pending_notice_records} pending_completions=${terminalStream.pending_completion_records} trimmed_recipients=${terminalStream.trimmed_pending_output_recipients} limit=${terminalStream.pending_output_record_limit_per_attachment}`,
    `slices: total=${sliceLifecycle.total_slices} running=${sliceLifecycle.running_slices} starting=${sliceLifecycle.starting_slices} stopping=${sliceLifecycle.stopping_slices} stopped=${sliceLifecycle.stopped_slices} unhealthy=${sliceLifecycle.unhealthy_slices} agents=${sliceLifecycle.attached_agents} failed_ops=${sliceLifecycle.failed_operations} in_progress_ops=${sliceLifecycle.in_progress_operations} auth_missing=${sliceLifecycle.provider_auth_missing_slices} auth_unconfigured=${sliceLifecycle.provider_auth_unconfigured_slices}`,
    `remote execution: remote_agents=${remoteExecution.remote_agents} active=${remoteExecution.active_remote_agents} missing_worker_runs=${remoteExecution.missing_active_worker_runs} malformed=${remoteExecution.malformed_bindings}`,
    `remote extensions: remote_agents=${remoteExtensionSync.remote_agents} home_proxy_agents=${remoteExtensionSync.home_proxy_agents} grants=${remoteExtensionSync.home_proxy_grants} synced=${remoteExtensionSync.synced_agents} syncing=${remoteExtensionSync.syncing_agents} pending=${remoteExtensionSync.pending_agents} failed=${remoteExtensionSync.failed_agents} stale=${remoteExtensionSync.stale_agents} missing=${remoteExtensionSync.manifest_missing_agents} pending_revoke=${remoteExtensionSync.pending_revoke_agents}`,
    `workspace coordination: claims=${workspaceCoordination.active_worktree_claims.length} collisions=${workspaceCoordination.worktree_collisions.length} active_ops=${workspaceCoordination.active_operation_claims.length}`,
    `workspace live sync: reservations=${liveSync.active_reservations} artifacts=${liveSync.active_reservation_artifacts} managed_write_fence=${liveSyncManagedMode.write_fence_supported ? "yes" : "no"} backend=${liveSyncManagedMode.write_fence_backend ?? "-"} tracked_runs=${workspaceIdentity.tracked_provider_runs} identity_changed=${workspaceIdentity.identity_changed_provider_runs} invalid_runs=${workspaceIdentity.invalid_provider_runs}`,
    `workspace watcher: tracked=${externalChanges.tracked_artifacts} external_changes=${externalChanges.externally_changed_artifacts} events=${externalChanges.external_change_events} scans=${externalChanges.live_watcher_scans} scan_errors=${externalChanges.live_watcher_scan_errors} started=${externalChanges.live_watcher_started ? "yes" : "no"}`,
  ]

  if (
    providerRuns.duplicate_arroba_agent_bindings.length === 0
    && providerRuns.multi_interface_agent_bindings.length === 0
  ) {
    lines.push("provider run bindings: ok")
  } else {
    lines.push("duplicate Arroba provider run bindings:")
    for (const conflict of providerRuns.duplicate_arroba_agent_bindings) {
      lines.push(`  session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
    }
    lines.push("  next: inspect the agent and stop duplicate provider runs before sending more prompts")
  }

  if (providerRuns.multi_interface_agent_bindings.length > 0) {
    lines.push("multi-interface provider run bindings:")
    for (const conflict of providerRuns.multi_interface_agent_bindings) {
      lines.push(`  session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
    }
    lines.push("  next: inspect the agent and close the extra native TUI or Arroba provider run before sending more prompts")
  }

  if (providerCatalog.expired) {
    lines.push(`provider catalog is stale: age=${formatDuration(providerCatalog.age_ms ?? null)} ttl=${formatDuration(providerCatalog.ttl_ms)}`)
    lines.push("  next: refresh provider/model selection before launching new sessions or agents")
  }

  if (providerRuns.orphaned_active_runs.length > 0) {
    lines.push("orphaned active provider runs:")
    for (const issue of providerRuns.orphaned_active_runs) {
      lines.push(`  run=${issue.provider_run_id} session=${issue.session_id} agent=${issue.agent_id ?? "-"}: ${issue.details}`)
    }
    lines.push("  next: refresh the session; stop or relaunch the orphaned provider run if it stays active")
  }

  if (providerRuns.session_active_run_mismatches.length > 0) {
    lines.push("session active provider run pointer issues:")
    for (const issue of providerRuns.session_active_run_mismatches) {
      lines.push(`  session=${issue.session_id} active=${issue.active_provider_run_id ?? "-"}: ${issue.details}`)
    }
    lines.push("  next: inspect the session and relaunch the affected agent to restore one active run pointer")
  }

  if (providerRunActor.enqueue_rejections > 0) {
    lines.push(`provider run actor rejected ${providerRunActor.enqueue_rejections} command${providerRunActor.enqueue_rejections === 1 ? "" : "s"}`)
    lines.push("  next: wait for provider-run command queues to drain; inspect duplicate/stuck provider runs if rejections continue")
  }

  if (commandLanes.saturated > 0) {
    lines.push(`command lane saturation: ${commandLanes.saturated} lane${commandLanes.saturated === 1 ? "" : "s"} at capacity`)
    for (const lane of commandLanes.saturatedLanes.slice(0, 8)) {
      lines.push(`  ${lane.kind} lane=${lane.laneId} queued=${lane.queued}/${lane.limit}`)
    }
    if (commandLanes.saturatedLanes.length > 8) {
      lines.push(`  ${commandLanes.saturatedLanes.length - 8} more saturated lane${commandLanes.saturatedLanes.length - 8 === 1 ? "" : "s"}`)
    }
    lines.push("  next: wait for active operations to drain; inspect stuck sessions/agents if saturation persists")
  }

  if (capability.rejected_jobs > 0 || capability.join_errors > 0) {
    lines.push(`capability executor issues: rejected=${capability.rejected_jobs} join_errors=${capability.join_errors}`)
  }

  const transportIssueCount = transportHealthIssueCount(health)
  if (transportIssueCount > 0) {
    lines.push(`transport issues: replay_gaps=${transport.replay_gaps} overloads=${transport.inbound_overload_rejections} duplicate_commands=${transport.duplicate_command_conflicts} outgoing_overflows=${transport.outgoing_queue_overflows} slow_consumers=${transport.slow_consumer_closes}`)
    lines.push("  next: reconnect stale clients; if overloads persist, reduce concurrent clients or restart the affected relay/kernel")
  }

  if (terminalStream.trimmed_pending_output_recipients > 0) {
    lines.push(`terminal stream trimmed pending output for ${terminalStream.trimmed_pending_output_recipients} recipient${terminalStream.trimmed_pending_output_recipients === 1 ? "" : "s"}`)
    lines.push("  next: refresh the terminal session to receive a fresh projection snapshot")
  }

  if (sliceLifecycle.unhealthy_slices > 0 || sliceLifecycle.failed_operations > 0) {
    lines.push(`slice lifecycle issues: unhealthy=${sliceLifecycle.unhealthy_slices} failed_ops=${sliceLifecycle.failed_operations}`)
    for (const issue of sliceLifecycle.issues) {
      const operation = issue.last_operation ? ` op=${issue.last_operation}` : ""
      const operationStatus = issue.last_operation_status ? ` op_status=${issue.last_operation_status}` : ""
      const worktree = issue.worktree_id ? ` worktree=${issue.worktree_id}` : ""
      const agents = issue.agent_ids.length > 0 ? ` agents=${issue.agent_ids.join(",")}` : ""
      const error = issue.last_error ? `: ${issue.last_error}` : ""
      lines.push(`  slice=${issue.name} (${issue.slice_id}) status=${issue.status}${operation}${operationStatus}${worktree}${agents}${error}`)
    }
    lines.push("  next: run /slice doctor for the affected slice, then inspect logs or restart/delete the slice")
  }

  if (sliceLifecycle.provider_auth_issues.length > 0) {
    lines.push(`slice provider auth issues: missing=${sliceLifecycle.provider_auth_missing_slices} unconfigured=${sliceLifecycle.provider_auth_unconfigured_slices}`)
    for (const issue of sliceLifecycle.provider_auth_issues) {
      const provider = issue.provider ? ` provider=${issue.provider}` : ""
      const state = issue.provider_auth_state ? ` state=${issue.provider_auth_state}` : ""
      const alias = issue.alias ? ` alias=${issue.alias}` : ""
      const identity = issue.identity ? ` identity=${issue.identity}` : ""
      const worktree = issue.worktree_id ? ` worktree=${issue.worktree_id}` : ""
      const agents = issue.agent_ids.length > 0 ? ` agents=${issue.agent_ids.join(",")}` : ""
      lines.push(`  slice=${issue.name} (${issue.slice_id}) status=${issue.status}${worktree}${agents}${provider}${state}${alias}${identity}: ${issue.details}`)
    }
    lines.push("  next: run /slice doctor for the affected slice; use /slice auth login or /slice auth import before sending more provider prompts")
  } else if (sliceLifecycle.provider_auth_missing_slices > 0 || sliceLifecycle.provider_auth_unconfigured_slices > 0) {
    lines.push(`slice provider auth issues: missing=${sliceLifecycle.provider_auth_missing_slices} unconfigured=${sliceLifecycle.provider_auth_unconfigured_slices}`)
    lines.push("  next: run /slice doctor for the affected slice; use /slice auth login or /slice auth import before sending more provider prompts")
  }

  if (remoteExecutionIssueCount(health) > 0) {
    lines.push(`remote execution issues: missing_worker_runs=${remoteExecution.missing_active_worker_runs} malformed=${remoteExecution.malformed_bindings}`)
    const affectedAgents = new Set<string>()
    for (const issue of remoteExecution.issues) {
      const workerRun = issue.active_worker_provider_run_id ? ` worker_run=${issue.active_worker_provider_run_id}` : ""
      const worktree = issue.worktree_id ? ` worktree=${issue.worktree_id}` : ""
      affectedAgents.add(issue.agent_ref || issue.agent_id)
      lines.push(`  agent=${issue.agent_ref} (${issue.agent_id}) session=${issue.session_id} worker=${issue.worker_kernel_id}/${issue.worker_machine_id} lease=${issue.execution_lease_id} leased_agent=${issue.leased_agent_id}${workerRun} state=${issue.state} processing=${issue.is_processing ? "yes" : "no"} kind=${issue.kind}${worktree}: ${issue.details}`)
    }
    const firstAgent = [...affectedAgents].find((agent) => agent.length > 0)
    const inspect = firstAgent ? `run /agent inspect ${firstAgent}` : "inspect the agent placement"
    lines.push(`  next: ${inspect}; reconnect or relaunch the remote/slice worker before sending more prompts`)
  }

  if (remoteExtensionSyncIssueCount(health) > 0) {
    lines.push(`remote extension sync issues: failed=${remoteExtensionSync.failed_agents} stale=${remoteExtensionSync.stale_agents} missing=${remoteExtensionSync.manifest_missing_agents} pending_revoke=${remoteExtensionSync.pending_revoke_agents}`)
    for (const issue of remoteExtensionSync.issues) {
      const workerRun = issue.active_worker_provider_run_id ? ` worker_run=${issue.active_worker_provider_run_id}` : ""
      const pendingRevoke = issue.pending_revoke ? " pending_revoke=yes" : ""
      const hash = issue.manifest_hash ? ` hash=${issue.manifest_hash}` : ""
      const worktree = issue.worktree_id ? ` worktree=${issue.worktree_id}` : ""
      const grants = issue.home_proxy_grants.length > 0 ? ` grants=${issue.home_proxy_grants.join(",")}` : ""
      const error = issue.last_error ? `: ${issue.last_error}` : ""
      lines.push(`  agent=${issue.agent_ref} (${issue.agent_id}) session=${issue.session_id} worker=${issue.worker_kernel_id}/${issue.worker_machine_id} lease=${issue.execution_lease_id} leased_agent=${issue.leased_agent_id}${workerRun} state=${issue.state}${pendingRevoke}${hash}${worktree}${grants}${error}`)
    }
    lines.push("  next: run /extension sync-status <agent>; use /extension sync-retry <agent> after worker connectivity is healthy")
  }

  if (workspaceCoordination.worktree_collisions.length > 0) {
    lines.push("workspace worktree collisions:")
    for (const collision of workspaceCoordination.worktree_collisions) {
      lines.push(`  workspace=${collision.workspace_id} worktree=${collision.worktree_id} sessions=${collision.session_ids.join(",")}`)
    }
    lines.push("  next: move one session/agent to a different worktree or intentionally bind the worktrees with workspace live sync")
  }

  if (workspaceCoordination.active_operation_claims.length > 0) {
    lines.push("workspace active operations:")
    for (const claim of workspaceCoordination.active_operation_claims) {
      lines.push(`  ${claim.mode} ${claim.operation} workspace=${claim.workspace_id} worktree=${claim.worktree_id} session=${claim.session_id}${claim.attachment_id ? ` attachment=${claim.attachment_id}` : ""}`)
    }
  }

  if (workspaceIdentity.identity_changed_provider_runs > 0 || workspaceIdentity.invalid_provider_runs > 0) {
    lines.push(`workspace identity issues: changed=${workspaceIdentity.identity_changed_provider_runs} invalid=${workspaceIdentity.invalid_provider_runs}`)
    for (const issue of workspaceIdentity.issues) {
      const branch = formatIdentityTransition(issue.baseline_branch, issue.current_branch)
      const head = formatIdentityTransition(issue.baseline_head_commit, issue.current_head_commit)
      const repo = formatIdentityTransition(issue.baseline_repo_url, issue.current_repo_url)
      lines.push(`  run=${issue.provider_run_id} root=${issue.root} generation=${issue.generation} valid=${issue.valid ? "yes" : "no"} fingerprint=${issue.baseline_fingerprint}->${issue.current_fingerprint} branch=${branch} head=${head} repo=${repo}`)
    }
    lines.push("  next: stop and relaunch affected managed/tracked provider runs after confirming the selected worktree")
  }

  if (externalChanges.issues.length > 0) {
    lines.push("workspace external changes:")
    for (const issue of externalChanges.issues) {
      const providerRun = issue.provider_run_id ?? "-"
      const root = issue.workspace_root ?? "-"
      lines.push(`  run=${providerRun} root=${root} path=${issue.path} fingerprint=${issue.workspace_fingerprint}`)
    }
    lines.push("  next: inspect the path; rerun or reconcile the affected managed/tracked turn before retrying workspace live sync")
  }

  if (!liveSyncManagedMode.write_fence_supported && liveSyncManagedMode.unavailable_reason) {
    lines.push(`workspace live sync managed mode unavailable: ${liveSyncManagedMode.unavailable_reason}`)
    lines.push("  next: select tracked mode on this worker or run the managed provider on a supported host")
  }

  if (externalChanges.live_watcher_scan_errors > 0) {
    lines.push(`workspace watcher scan errors: ${externalChanges.live_watcher_scan_errors}`)
    lines.push("  next: check workspace paths and permissions, then refresh workspace live sync status")
  }

  if (health.projection_invariants.mismatches.length === 0) {
    lines.push(`projection invariants: ok (${health.projection_invariants.checked_sessions} session${health.projection_invariants.checked_sessions === 1 ? "" : "s"}, ${health.projection_invariants.checked_agents} agent${health.projection_invariants.checked_agents === 1 ? "" : "s"})`)
  } else {
    lines.push("projection invariant mismatches:")
    for (const mismatch of health.projection_invariants.mismatches) {
      lines.push(`  ${mismatch.kind} session=${mismatch.session_id} agent=${mismatch.agent_id ?? "-"}: ${mismatch.details}`)
    }
    lines.push("  next: refresh the session; restart the kernel if the invariant mismatch persists")
  }

  return lines.join("\n")
}

function providerCatalogHealthIssueCount(health: DaemonHealthProjection): number {
  return health.provider_catalog.expired ? 1 : 0
}

function workspaceHealthIssueCount(health: DaemonHealthProjection): number {
  const externalChanges = health.workspace_live_sync.external_changes
  const managedMode = health.workspace_live_sync.managed_mode
  return health.workspace_coordination.worktree_collisions.length
    + health.workspace_live_sync.workspace_identity.identity_changed_provider_runs
    + health.workspace_live_sync.workspace_identity.invalid_provider_runs
    + (externalChanges.issues.length > 0
      ? externalChanges.issues.length
      : externalChanges.externally_changed_artifacts)
    + externalChanges.live_watcher_scan_errors
    + (!managedMode.write_fence_supported && managedMode.unavailable_reason ? 1 : 0)
}

function transportHealthIssueCount(health: DaemonHealthProjection): number {
  const transport = health.transport
  return transport.replay_gaps
    + transport.inbound_overload_rejections
    + transport.duplicate_command_conflicts
    + transport.outgoing_queue_overflows
    + transport.slow_consumer_closes
}

function terminalStreamHealthIssueCount(health: DaemonHealthProjection): number {
  return health.terminal_stream.trimmed_pending_output_recipients
}

function sliceLifecycleIssueCount(health: DaemonHealthProjection): number {
  const lifecycleIssues = health.slice_lifecycle.issues.length > 0
    ? health.slice_lifecycle.issues.length
    : health.slice_lifecycle.unhealthy_slices + health.slice_lifecycle.failed_operations
  const providerAuthIssues = health.slice_lifecycle.provider_auth_issues.length > 0
    ? health.slice_lifecycle.provider_auth_issues.length
    : health.slice_lifecycle.provider_auth_missing_slices + health.slice_lifecycle.provider_auth_unconfigured_slices
  return lifecycleIssues + providerAuthIssues
}

function remoteExecutionIssueCount(health: DaemonHealthProjection): number {
  return health.remote_execution.issues.length > 0
    ? health.remote_execution.issues.length
    : health.remote_execution.missing_active_worker_runs + health.remote_execution.malformed_bindings
}

function remoteExtensionSyncIssueCount(health: DaemonHealthProjection): number {
  return health.remote_extension_sync.issues.length > 0
    ? health.remote_extension_sync.issues.length
    : (
      health.remote_extension_sync.failed_agents
      + health.remote_extension_sync.stale_agents
      + health.remote_extension_sync.manifest_missing_agents
      + health.remote_extension_sync.pending_revoke_agents
    )
}

function capabilityHealthIssueCount(health: DaemonHealthProjection): number {
  return health.capability_executor.rejected_jobs + health.capability_executor.join_errors
}

function providerRunActorHealthIssueCount(health: DaemonHealthProjection): number {
  return health.provider_run_actor.enqueue_rejections
}

function commandLaneHealthIssueCount(health: DaemonHealthProjection): number {
  return commandLaneHealthSummary(health).saturated
}

function commandLaneHealthSummary(health: DaemonHealthProjection): {
  session: CommandLaneKindSummary
  agent: CommandLaneKindSummary
  workflow: CommandLaneKindSummary
  provider: CommandLaneKindSummary
  saturated: number
  saturatedLanes: CommandLaneIssue[]
} {
  const session = summarizeCommandLanes("session", health.session_command_lanes)
  const agent = summarizeCommandLanes("agent", health.agent_command_lanes)
  const workflow = summarizeCommandLanes("workflow", health.workflow_command_lanes)
  const provider = summarizeCommandLanes("provider", health.provider_runtime_lanes)
  const saturatedLanes = [
    ...session.saturatedLanes,
    ...agent.saturatedLanes,
    ...workflow.saturatedLanes,
    ...provider.saturatedLanes,
  ]
  return {
    session,
    agent,
    workflow,
    provider,
    saturated: saturatedLanes.length,
    saturatedLanes,
  }
}

type CommandLaneKind = "session" | "agent" | "workflow" | "provider"

type CommandLaneKindSummary = {
  lanes: number
  queued: number
  saturatedLanes: CommandLaneIssue[]
}

type CommandLaneIssue = {
  kind: CommandLaneKind
  laneId: string
  queued: number
  limit: number
}

function summarizeCommandLanes(
  kind: CommandLaneKind,
  lanes: readonly { lane_id: string; queue_limit: number; queued_commands: number }[],
): CommandLaneKindSummary {
  const saturatedLanes = lanes
    .filter((lane) => lane.queue_limit > 0 && lane.queued_commands >= lane.queue_limit)
    .map((lane) => ({
      kind,
      laneId: lane.lane_id,
      queued: lane.queued_commands,
      limit: lane.queue_limit,
    }))
  return {
    lanes: lanes.length,
    queued: lanes.reduce((sum, lane) => sum + lane.queued_commands, 0),
    saturatedLanes,
  }
}

function formatBytes(bytes: number | null): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) {
    return "unknown"
  }
  const units = ["B", "KiB", "MiB", "GiB"]
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const formatted = unitIndex === 0 ? `${Math.round(value)}` : value >= 10 ? value.toFixed(1) : value.toFixed(2)
  return `${formatted}${units[unitIndex]}`
}

function formatDuration(ms: number | null): string {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) {
    return "unknown"
  }
  if (ms < 1000) {
    return `${Math.round(ms)}ms`
  }
  const seconds = ms / 1000
  if (seconds < 60) {
    return `${seconds >= 10 ? seconds.toFixed(1) : seconds.toFixed(2)}s`
  }
  const minutes = seconds / 60
  return `${minutes >= 10 ? minutes.toFixed(1) : minutes.toFixed(2)}m`
}

function formatIdentityTransition(before: string | null | undefined, after: string | null | undefined): string {
  return `${before || "-"}->${after || "-"}`
}
