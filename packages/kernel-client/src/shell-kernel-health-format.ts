import type { DaemonHealthProjection } from "./kernel-types.js"
import { remoteWorkerProviderRunRecoveryAction } from "./provider-run-recovery.js"
import { remoteExtensionSyncNextAction } from "./shell-capability-format.js"

export function kernelHealthIssueCount(health: DaemonHealthProjection): number {
  return duplicateProviderRunBindingCount(health)
    + health.provider_runs.multi_interface_agent_bindings.length
    + health.provider_runs.orphaned_active_runs.length
    + health.provider_runs.session_active_run_mismatches.length
    + health.provider_runs.terminal_diagnostics.length
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

export function kernelRemoteRuntimeIssueCount(health: DaemonHealthProjection): number {
  return remoteRuntimeHardIssueCount(health)
}

export type KernelRemoteRuntimeReadiness = {
  readonly state: "ok" | "degraded" | "blocked"
  readonly issueCount: number
  readonly attentionCount: number
}

export function kernelRemoteRuntimeReadiness(health: DaemonHealthProjection): KernelRemoteRuntimeReadiness {
  const issueCount = remoteRuntimeHardIssueCount(health)
  const attentionCount = issueCount + remoteRuntimeSettlingAttentionCount(health)
  return {
    state: issueCount > 0 ? "blocked" : attentionCount > 0 ? "degraded" : "ok",
    issueCount,
    attentionCount,
  }
}

function remoteRuntimeHardIssueCount(health: DaemonHealthProjection): number {
  return providerRunRuntimeIssueCount(health)
    + health.projection_invariants.mismatches.length
    + workspaceRemoteRuntimeHardIssueCount(health)
    + sliceLifecycleIssueCount(health)
    + remoteExecutionIssueCount(health)
    + remoteExtensionSyncHardIssueCount(health)
}

function remoteRuntimeSettlingAttentionCount(health: DaemonHealthProjection): number {
  const sliceOperations = Math.max(
    health.slice_lifecycle.starting_slices + health.slice_lifecycle.stopping_slices,
    health.slice_lifecycle.in_progress_operations,
  )
  return sliceOperations
    + health.remote_extension_sync.stale_agents
    + health.remote_extension_sync.pending_agents
    + health.remote_extension_sync.syncing_agents
    + health.workspace_live_sync.external_changes.live_watcher_scan_errors
}

function remoteRuntimeDegradedNextAction(health: DaemonHealthProjection): string {
  const sliceLifecycle = health.slice_lifecycle
  if (sliceLifecycle.starting_slices > 0 || sliceLifecycle.stopping_slices > 0 || sliceLifecycle.in_progress_operations > 0) {
    return "wait for slice operations to settle; if they remain in progress, run /slice list, then /slice doctor for the affected slice"
  }
  const remoteExtensionSync = health.remote_extension_sync
  if (remoteExtensionSync.pending_agents > 0 || remoteExtensionSync.syncing_agents > 0 || remoteExtensionSync.stale_agents > 0) {
    return "run /extension sync-status for affected agents; use /extension sync-retry after worker connectivity is healthy"
  }
  if (health.workspace_live_sync.external_changes.live_watcher_scan_errors > 0) {
    return "run /workspace sync status, then /workspace sync ignore; check selected workspace paths and permissions"
  }
  return "run /kernel remote-runtime again after current runtime operations settle"
}

function appendRemoteRuntimeReadiness(
  lines: string[],
  health: DaemonHealthProjection,
  { includeSupportBundle }: { readonly includeSupportBundle: boolean },
): void {
  const readiness = kernelRemoteRuntimeReadiness(health)
  if (readiness.state === "ok") {
    lines.push("remote runtime readiness: ok")
    return
  }
  if (readiness.state === "degraded") {
    lines.push(`remote runtime readiness: degraded (${readiness.attentionCount} attention)`)
    appendRemoteRuntimeAffectedTargets(lines, health)
    lines.push(`remote runtime readiness next: ${remoteRuntimeDegradedNextAction(health)}`)
    return
  }
  lines.push(`remote runtime readiness: blocked (${readiness.issueCount} issue${readiness.issueCount === 1 ? "" : "s"}, ${readiness.attentionCount} attention)`)
  appendRemoteRuntimeAffectedTargets(lines, health)
  lines.push(`remote runtime readiness next: ${remoteRuntimeBlockedNextAction(health)}`)
  if (includeSupportBundle) {
    lines.push("support bundle: after reproducing, run /kernel debug-bundle <label> from TUI or kernel debug-bundle <label> from arroba-shell")
  }
}

function appendRemoteRuntimeAffectedTargets(lines: string[], health: DaemonHealthProjection): void {
  const affected = remoteRuntimeAffectedTargets(health)
  const parts = [
    affected.agents.length > 0 ? `agents=${formatAffectedTargets(affected.agents)}` : null,
    affected.worktrees.length > 0 ? `worktrees=${formatAffectedTargets(affected.worktrees)}` : null,
    affected.roots.length > 0 ? `roots=${formatAffectedTargets(affected.roots)}` : null,
  ].filter((part): part is string => Boolean(part))
  if (parts.length > 0) {
    lines.push(`remote runtime affected: ${parts.join(" ")}`)
  }
}

function remoteRuntimeAffectedTargets(health: DaemonHealthProjection): {
  readonly agents: string[]
  readonly worktrees: string[]
  readonly roots: string[]
} {
  const agents = [
    ...health.remote_execution.issues.map((issue) => issue.agent_ref || issue.agent_id),
    ...health.remote_extension_sync.issues.map((issue) => issue.agent_ref || issue.agent_id),
    ...health.slice_lifecycle.issues.flatMap((issue) => issue.agent_ids),
    ...health.slice_lifecycle.provider_auth_issues.flatMap((issue) => issue.agent_ids),
  ]
  const worktrees = [
    ...health.remote_execution.issues.map((issue) => issue.worktree_id),
    ...health.remote_extension_sync.issues.map((issue) => issue.worktree_id),
    ...health.slice_lifecycle.issues.map((issue) => issue.worktree_id),
    ...health.slice_lifecycle.provider_auth_issues.map((issue) => issue.worktree_id),
    ...health.workspace_coordination.worktree_collisions.map((issue) => issue.worktree_id),
    ...health.workspace_coordination.active_operation_claims.map((issue) => issue.worktree_id),
  ]
  const roots = [
    ...health.workspace_live_sync.workspace_identity.issues.map((issue) => issue.root),
    ...health.workspace_live_sync.external_changes.issues.map((issue) => issue.workspace_root),
  ]
  return {
    agents: uniqueNonEmpty(agents.filter(nonEmptyRoot)),
    worktrees: uniqueNonEmpty(worktrees.filter(nonEmptyRoot)),
    roots: uniqueNonEmpty(roots.filter(nonEmptyRoot)),
  }
}

function formatAffectedTargets(values: readonly string[]): string {
  const shown = values.slice(0, 4).join(",")
  const more = values.length > 4 ? `,+${values.length - 4} more` : ""
  return `${shown}${more}`
}

function remoteRuntimeBlockedNextAction(health: DaemonHealthProjection): string {
  if (providerRunRuntimeIssueCount(health) > 0) {
    return "run /provider processes and /agent inspect for the affected agent; close or relaunch duplicate, orphaned, or mismatched provider runs"
  }
  if (health.projection_invariants.mismatches.length > 0) {
    return "refresh the affected session or agent projection; capture a debug bundle if the mismatch persists"
  }
  if (workspaceRemoteRuntimeHardIssueCount(health) > 0) {
    return "run /workspace sync status, /workspace sync targets, and /workspace sync conflicts to reconcile selected-workspace state"
  }
  if (sliceLifecycleIssueCount(health) > 0) {
    return "run /slice doctor for the affected slice, then inspect /slice logs and /slice audit before restarting or deleting it"
  }
  if (remoteExecutionIssueCount(health) > 0) {
    return "run /machine kernels for the affected worker, then relaunch or reconnect the remote provider run"
  }
  if (remoteExtensionSyncHardIssueCount(health) > 0) {
    return "run /extension sync-status for affected agents; use /extension sync-retry after worker connectivity is healthy"
  }
  return "run /kernel health and capture a debug bundle for the affected remote runtime surface"
}

export function formatKernelRemoteRuntimeHealth(health: DaemonHealthProjection): string {
  const providerRuns = health.provider_runs
  const sliceLifecycle = health.slice_lifecycle
  const remoteExecution = health.remote_execution
  const remoteExtensionSync = health.remote_extension_sync
  const workspaceCoordination = health.workspace_coordination
  const liveSync = health.workspace_live_sync
  const liveSyncManagedMode = liveSync.managed_mode
  const workspaceIdentity = liveSync.workspace_identity
  const externalChanges = liveSync.external_changes
  const liveSyncAffectedRoots = workspaceLiveSyncAffectedRoots(health)
  const providerRunIssues = providerRunInvariantIssueCount(health)
  const lines = [
    "remote runtime",
    `provider runs: projected=${providerRuns.projected_runs} active=${providerRuns.active_runs} arroba=${providerRuns.arroba_active_runs} native_tui=${providerRuns.native_tui_active_runs}`,
    `provider run invariants: duplicate=${duplicateProviderRunBindingCount(health)} mixed=${providerRuns.multi_interface_agent_bindings.length} orphaned=${providerRuns.orphaned_active_runs.length} pointer=${providerRuns.session_active_run_mismatches.length} terminal=${providerRuns.terminal_diagnostics.length} actor_rejects=${health.provider_run_actor.enqueue_rejections}`,
    `remote execution: remote_agents=${remoteExecution.remote_agents} active=${remoteExecution.active_remote_agents} missing_worker_runs=${remoteExecution.missing_active_worker_runs} malformed=${remoteExecution.malformed_bindings}`,
    `slices: total=${sliceLifecycle.total_slices} running=${sliceLifecycle.running_slices} starting=${sliceLifecycle.starting_slices} stopping=${sliceLifecycle.stopping_slices} stopped=${sliceLifecycle.stopped_slices} unhealthy=${sliceLifecycle.unhealthy_slices} agents=${sliceLifecycle.attached_agents} failed_ops=${sliceLifecycle.failed_operations} in_progress_ops=${sliceLifecycle.in_progress_operations} auth_missing=${sliceLifecycle.provider_auth_missing_slices} auth_unconfigured=${sliceLifecycle.provider_auth_unconfigured_slices}`,
    `remote extensions: remote_agents=${remoteExtensionSync.remote_agents} home_proxy_agents=${remoteExtensionSync.home_proxy_agents} grants=${remoteExtensionSync.home_proxy_grants} synced=${remoteExtensionSync.synced_agents} syncing=${remoteExtensionSync.syncing_agents} pending=${remoteExtensionSync.pending_agents} failed=${remoteExtensionSync.failed_agents} stale=${remoteExtensionSync.stale_agents} missing=${remoteExtensionSync.manifest_missing_agents} pending_revoke=${remoteExtensionSync.pending_revoke_agents}`,
    `session projection: checked_sessions=${health.projection_invariants.checked_sessions} checked_agents=${health.projection_invariants.checked_agents} mismatches=${health.projection_invariants.mismatches.length}`,
    ...(remoteExtensionSync.home_proxy_agents > 0
      ? ["remote extension runtime: home owns grants, credentials, and execution; workers receive projected manifests only"]
      : []),
    formatRemoteRuntimeInvariantSummary(health),
    `workspace coordination: claims=${workspaceCoordination.active_worktree_claims.length} collisions=${workspaceCoordination.worktree_collisions.length} active_ops=${workspaceCoordination.active_operation_claims.length}`,
    `workspace live sync: reservations=${liveSync.active_reservations} artifacts=${liveSync.active_reservation_artifacts} managed_write_fence=${liveSyncManagedMode.write_fence_supported ? "yes" : "no"} backend=${liveSyncManagedMode.write_fence_backend ?? "-"} tracked_runs=${workspaceIdentity.tracked_provider_runs} identity_changed=${workspaceIdentity.identity_changed_provider_runs} invalid_runs=${workspaceIdentity.invalid_provider_runs}`,
    `workspace live sync scope: ${workspaceLiveSyncScopeDetail(liveSyncAffectedRoots)}`,
    `workspace watcher: tracked=${externalChanges.tracked_artifacts} external_changes=${externalChanges.externally_changed_artifacts} events=${externalChanges.external_change_events} scans=${externalChanges.live_watcher_scans} scan_errors=${externalChanges.live_watcher_scan_errors} started=${externalChanges.live_watcher_started ? "yes" : "no"}`,
  ]

  if (providerRunIssues > 0 || health.provider_run_actor.enqueue_rejections > 0) {
    appendRemoteRuntimeProviderRunIssues(lines, health)
  }
  if (health.projection_invariants.mismatches.length > 0) {
    appendRemoteRuntimeProjectionInvariantIssues(lines, health)
  }
  appendRemoteRuntimeIssues(lines, health)
  appendRemoteRuntimeReadiness(lines, health, { includeSupportBundle: true })
  return lines.join("\n")
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
  const liveSyncAffectedRoots = workspaceLiveSyncAffectedRoots(health)
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
    ...(remoteExtensionSync.home_proxy_agents > 0
      ? ["remote extension runtime: home owns grants, credentials, and execution; workers receive projected manifests only"]
      : []),
    formatRemoteRuntimeInvariantSummary(health),
    `workspace coordination: claims=${workspaceCoordination.active_worktree_claims.length} collisions=${workspaceCoordination.worktree_collisions.length} active_ops=${workspaceCoordination.active_operation_claims.length}`,
    `workspace live sync: reservations=${liveSync.active_reservations} artifacts=${liveSync.active_reservation_artifacts} managed_write_fence=${liveSyncManagedMode.write_fence_supported ? "yes" : "no"} backend=${liveSyncManagedMode.write_fence_backend ?? "-"} tracked_runs=${workspaceIdentity.tracked_provider_runs} identity_changed=${workspaceIdentity.identity_changed_provider_runs} invalid_runs=${workspaceIdentity.invalid_provider_runs}`,
    `workspace live sync scope: ${workspaceLiveSyncScopeDetail(liveSyncAffectedRoots)}`,
    `workspace watcher: tracked=${externalChanges.tracked_artifacts} external_changes=${externalChanges.externally_changed_artifacts} events=${externalChanges.external_change_events} scans=${externalChanges.live_watcher_scans} scan_errors=${externalChanges.live_watcher_scan_errors} started=${externalChanges.live_watcher_started ? "yes" : "no"}`,
  ]

  const providerRunInvariantIssues =
    providerRunInvariantIssueCount(health)
  if (providerRunInvariantIssues === 0) {
    lines.push("provider run invariants: ok")
  }

  if (providerRuns.duplicate_arroba_agent_bindings.length > 0) {
    lines.push("duplicate Arroba provider run bindings:")
    let firstAgent: string | null = null
    for (const conflict of providerRuns.duplicate_arroba_agent_bindings) {
      firstAgent ??= conflict.agent_id
      lines.push(`  session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
    }
    lines.push("  invariant: normal Arroba launches should replace idle same-agent runs instead of creating duplicates")
    lines.push(firstAgent
      ? `  next: run /agent inspect ${firstAgent}; run /provider processes; capture a debug bundle, then stop duplicate provider runs before sending prompts to that agent`
      : "  next: run /provider processes; capture a debug bundle, then identify and stop duplicate provider runs before sending more prompts")
  }

  if (providerRuns.duplicate_native_tui_agent_bindings.length > 0) {
    lines.push("duplicate native TUI provider run bindings:")
    let firstAgent: string | null = null
    for (const conflict of providerRuns.duplicate_native_tui_agent_bindings) {
      firstAgent ??= conflict.agent_id
      lines.push(`  session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
    }
    lines.push("  invariant: native TUI attachments should share one provider run per agent unless multi-run ownership is explicitly modeled")
    lines.push(firstAgent
      ? `  next: run /agent inspect ${firstAgent}; run /provider processes; close duplicate native TUIs before sending prompts to that agent`
      : "  next: run /provider processes; identify and close duplicate native TUIs before sending more prompts")
  }

  if (providerRuns.multi_interface_agent_bindings.length > 0) {
    lines.push("multi-interface provider run bindings:")
    let firstAgent: string | null = null
    for (const conflict of providerRuns.multi_interface_agent_bindings) {
      firstAgent ??= conflict.agent_id
      lines.push(`  session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
    }
    lines.push(firstAgent
      ? `  next: run /agent inspect ${firstAgent}; run /provider processes; close the extra native TUI or Arroba provider run before sending prompts to that agent`
      : "  next: run /provider processes; close the extra native TUI or Arroba provider run after identifying the affected agent")
  }

  if (providerCatalog.expired) {
    lines.push(`provider catalog is stale: age=${formatDuration(providerCatalog.age_ms ?? null)} ttl=${formatDuration(providerCatalog.ttl_ms)}`)
    lines.push("  next: refresh provider/model selection before launching new sessions or agents")
  }

  if (providerRuns.orphaned_active_runs.length > 0) {
    lines.push("orphaned active provider runs:")
    let firstRun: string | null = null
    for (const issue of providerRuns.orphaned_active_runs) {
      firstRun ??= issue.provider_run_id
      lines.push(`  run=${issue.provider_run_id} session=${issue.session_id} agent=${issue.agent_id ?? "-"}: ${issue.details}`)
    }
    const target = firstRun ? `provider run ${firstRun}` : "the orphaned provider run"
    lines.push(`  next: refresh the session; stop or relaunch ${target} if it stays active`)
  }

  if (providerRuns.session_active_run_mismatches.length > 0) {
    lines.push("session active provider run pointer issues:")
    let firstSession: string | null = null
    for (const issue of providerRuns.session_active_run_mismatches) {
      firstSession ??= issue.session_id
      lines.push(`  session=${issue.session_id} active=${issue.active_provider_run_id ?? "-"}: ${issue.details}`)
    }
    const target = firstSession ? `session ${firstSession}` : "the affected session"
    lines.push(`  next: inspect ${target} and relaunch the affected agent to restore one active run pointer`)
  }

  if (providerRuns.terminal_diagnostics.length > 0) {
    lines.push("provider run terminal diagnostics:")
    let firstAgent: string | null = null
    let firstRun: string | null = null
    for (const issue of providerRuns.terminal_diagnostics) {
      firstAgent ??= issue.agent_id ?? null
      firstRun ??= issue.provider_run_id
      lines.push(`  run=${issue.provider_run_id} provider=${issue.provider} state=${issue.state} session=${issue.session_id} agent=${issue.agent_id ?? "-"}: ${issue.diagnostic}`)
    }
    const target = firstAgent
      ? `agent ${firstAgent}`
      : firstRun
        ? `provider run ${firstRun}`
        : "the affected provider run"
    const inspectAction = firstAgent
      ? `run /agent inspect ${firstAgent}; `
      : "identify the affected agent from /provider processes or the debug bundle; "
    lines.push(`  next: ${inspectAction}run /provider processes; relaunch ${target} if the diagnostic persists; capture a debug bundle before restarting the kernel`)
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
    lines.push(`  next: wait for active operations to drain; inspect ${commandLaneInspectionTargets(commandLanes.saturatedLanes)} if saturation persists`)
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

  appendRemoteRuntimeIssues(lines, health)
  appendRemoteRuntimeReadiness(lines, health, { includeSupportBundle: false })

  if (health.projection_invariants.mismatches.length === 0) {
    lines.push(`projection invariants: ok (${health.projection_invariants.checked_sessions} session${health.projection_invariants.checked_sessions === 1 ? "" : "s"}, ${health.projection_invariants.checked_agents} agent${health.projection_invariants.checked_agents === 1 ? "" : "s"})`)
  } else {
    lines.push("projection invariant mismatches:")
    for (const mismatch of health.projection_invariants.mismatches) {
      lines.push(`  ${mismatch.kind} session=${mismatch.session_id} agent=${mismatch.agent_id ?? "-"}: ${mismatch.details}`)
    }
    lines.push("  next: refresh the session; restart the kernel if the invariant mismatch persists")
  }

  if (kernelHealthIssueCount(health) > 0) {
    lines.push("support bundle: after reproducing, run /kernel debug-bundle <label> from TUI or kernel debug-bundle <label> from arroba-shell")
  }

  return lines.join("\n")
}

function appendRemoteRuntimeIssues(lines: string[], health: DaemonHealthProjection): void {
  const sliceLifecycle = health.slice_lifecycle
  const remoteExecution = health.remote_execution
  const remoteExtensionSync = health.remote_extension_sync
  const workspaceCoordination = health.workspace_coordination
  const liveSync = health.workspace_live_sync
  const liveSyncManagedMode = liveSync.managed_mode
  const workspaceIdentity = liveSync.workspace_identity
  const externalChanges = liveSync.external_changes

  if (
    sliceLifecycle.issues.length > 0
    || sliceLifecycle.unhealthy_slices > 0
    || sliceLifecycle.failed_operations > 0
  ) {
    lines.push(`slice lifecycle issues: unhealthy=${sliceLifecycle.unhealthy_slices} failed_ops=${sliceLifecycle.failed_operations}`)
    let hasStoppedSliceWithAgents = false
    for (const issue of sliceLifecycle.issues) {
      const operation = issue.last_operation ? ` op=${issue.last_operation}` : ""
      const operationStatus = issue.last_operation_status ? ` op_status=${issue.last_operation_status}` : ""
      const worktree = issue.worktree_id ? ` worktree=${issue.worktree_id}` : ""
      const agents = issue.agent_ids.length > 0 ? ` agents=${issue.agent_ids.join(",")}` : ""
      const stoppedWithAgents = issue.status === "stopped" && issue.agent_ids.length > 0
      hasStoppedSliceWithAgents ||= stoppedWithAgents
      const detail = issue.last_error
        ? `: ${issue.last_error}`
        : stoppedWithAgents
          ? ": stopped with attached agents"
          : ""
      lines.push(`  slice=${issue.name} (${issue.slice_id}) status=${issue.status}${operation}${operationStatus}${worktree}${agents}${detail}`)
    }
    const firstIssue = sliceLifecycle.issues[0]
    const firstSlice = firstIssue?.slice_id
    const sliceTarget = firstSlice ? ` ${firstSlice}` : ""
    const storageRecovery = firstIssue ? sliceStorageRecoveryAction(firstIssue.last_error) : ""
    lines.push(
      hasStoppedSliceWithAgents
        ? `  next: run /slice start${sliceTarget} for stopped slices or move attached agents to a running slice`
        : storageRecovery
          ? `  next: ${storageRecovery}; then run /slice start${sliceTarget} or recreate the slice if startup still fails`
        : sliceTarget
          ? `  next: run /slice doctor${sliceTarget}, inspect /slice logs${sliceTarget}, and check /slice audit${sliceTarget} before restarting or deleting the slice`
          : "  next: run /slice list to identify the affected slice, then run /slice doctor and inspect logs/audit before restarting or deleting it",
    )
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
    const firstIssue = sliceLifecycle.provider_auth_issues[0]
    const sliceRef = firstIssue?.slice_id
    const provider = firstIssue?.provider
    const providerAction = provider && sliceRef
      ? `use /slice auth login ${sliceRef} ${provider} or /slice auth import ${sliceRef} ${provider}`
      : provider
        ? `identify the affected slice, then use /slice auth login or /slice auth import for ${provider}`
        : "after provider discovery, use the matching /slice auth login or /slice auth import command"
    lines.push(sliceRef
      ? `  next: run /slice doctor ${sliceRef}; inspect /slice audit ${sliceRef}; ${providerAction} before sending prompts to agents in that slice`
      : `  next: run /slice doctor for the affected slice; inspect /slice audit; ${providerAction} before sending prompts to slice-backed agents`)
  } else if (sliceLifecycle.provider_auth_missing_slices > 0 || sliceLifecycle.provider_auth_unconfigured_slices > 0) {
    lines.push(`slice provider auth issues: missing=${sliceLifecycle.provider_auth_missing_slices} unconfigured=${sliceLifecycle.provider_auth_unconfigured_slices}`)
    lines.push("  next: run /slice list to identify affected slices; run /slice doctor and inspect /slice audit before choosing a provider account to login or import")
  }

  if (
    sliceLifecycle.issues.length === 0
    && sliceLifecycle.unhealthy_slices === 0
    && sliceLifecycle.failed_operations === 0
    && (sliceLifecycle.starting_slices > 0 || sliceLifecycle.stopping_slices > 0 || sliceLifecycle.in_progress_operations > 0)
  ) {
    lines.push(`slice operations settling: starting=${sliceLifecycle.starting_slices} stopping=${sliceLifecycle.stopping_slices} in_progress=${sliceLifecycle.in_progress_operations}`)
    lines.push("  next: wait for the slice operation to finish; run /slice list to identify any stuck slice, then run /slice doctor and inspect logs if it does not settle")
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
    const firstIssue = remoteExecution.issues[0]
    lines.push(`  next: ${remoteWorkerProviderRunRecoveryAction(firstAgent, firstIssue?.worker_machine_id)}`)
  }

  if (remoteExtensionSyncIssueCount(health) > 0) {
    lines.push(`remote extension sync issues: failed=${remoteExtensionSync.failed_agents} stale=${remoteExtensionSync.stale_agents} missing=${remoteExtensionSync.manifest_missing_agents} pending_revoke=${remoteExtensionSync.pending_revoke_agents}`)
    const affectedAgents = new Set<string>()
    for (const issue of remoteExtensionSync.issues) {
      const workerRun = issue.active_worker_provider_run_id ? ` worker_run=${issue.active_worker_provider_run_id}` : ""
      const pendingRevoke = issue.pending_revoke ? " pending_revoke=yes" : ""
      const hash = issue.manifest_hash ? ` hash=${issue.manifest_hash}` : ""
      const worktree = issue.worktree_id ? ` worktree=${issue.worktree_id}` : ""
      const grants = issue.home_proxy_grants.length > 0 ? ` grants=${issue.home_proxy_grants.join(",")}` : ""
      const error = issue.last_error ? `: ${issue.last_error}` : ""
      affectedAgents.add(issue.agent_ref || issue.agent_id)
      lines.push(`  agent=${issue.agent_ref} (${issue.agent_id}) session=${issue.session_id} worker=${issue.worker_kernel_id}/${issue.worker_machine_id} lease=${issue.execution_lease_id} leased_agent=${issue.leased_agent_id}${workerRun} state=${issue.state}${pendingRevoke}${hash}${worktree}${grants}${error}`)
    }
    const firstAgent = [...affectedAgents].find((agent) => agent.length > 0)
    const firstIssue = remoteExtensionSync.issues[0]
    const nextAction = firstAgent
      ? remoteExtensionSyncNextAction(firstIssue
        ? { state: firstIssue.state, pending_revoke: firstIssue.pending_revoke }
        : remoteExtensionSyncAggregateStatus(remoteExtensionSync), firstAgent, firstIssue?.worker_machine_id) ?? `run /extension sync-status ${firstAgent}`
      : remoteExtensionAggregateNextAction(remoteExtensionSync)
    lines.push(`  next: ${nextAction}`)
  }

  if (remoteExtensionSync.pending_agents > 0 || remoteExtensionSync.syncing_agents > 0) {
    lines.push(`remote extension sync settling: syncing=${remoteExtensionSync.syncing_agents} pending=${remoteExtensionSync.pending_agents}`)
    lines.push("  next: home keeps stale home-proxy calls blocked until worker manifests settle; run /kernel remote-runtime and then /extension sync-status for the affected agent before retrying sync")
  }

  if (workspaceCoordination.worktree_collisions.length > 0) {
    lines.push("workspace worktree collisions:")
    for (const collision of workspaceCoordination.worktree_collisions) {
      lines.push(`  workspace=${collision.workspace_id} worktree=${collision.worktree_id} sessions=${collision.session_ids.join(",")}`)
    }
    lines.push("  next: run /workspace sync targets and /workspace sync conflicts; move one session/agent to a different worktree or intentionally bind the worktrees")
  }

  if (workspaceCoordination.active_operation_claims.length > 0) {
    lines.push("workspace active operations:")
    for (const claim of workspaceCoordination.active_operation_claims) {
      lines.push(`  ${claim.mode} ${claim.operation} workspace=${claim.workspace_id} worktree=${claim.worktree_id} session=${claim.session_id}${claim.attachment_id ? ` attachment=${claim.attachment_id}` : ""}`)
    }
  }

  if (workspaceIdentity.identity_changed_provider_runs > 0 || workspaceIdentity.invalid_provider_runs > 0) {
    lines.push(`workspace identity issues: changed=${workspaceIdentity.identity_changed_provider_runs} invalid=${workspaceIdentity.invalid_provider_runs}`)
    let firstProviderRun: string | null = null
    for (const issue of workspaceIdentity.issues) {
      const branch = formatIdentityTransition(issue.baseline_branch, issue.current_branch)
      const head = formatIdentityTransition(issue.baseline_head_commit, issue.current_head_commit)
      const repo = formatIdentityTransition(issue.baseline_repo_url, issue.current_repo_url)
      firstProviderRun ??= issue.provider_run_id ?? null
      lines.push(`  run=${issue.provider_run_id} root=${issue.root} generation=${issue.generation} valid=${issue.valid ? "yes" : "no"} fingerprint=${issue.baseline_fingerprint}->${issue.current_fingerprint} branch=${branch} head=${head} repo=${repo}`)
    }
    const target = firstProviderRun ? `provider run ${firstProviderRun}` : "affected managed/tracked provider runs"
    lines.push(`  next: stop and relaunch ${target} after confirming the selected worktree`)
  }

  if (externalChanges.issues.length > 0) {
    lines.push("workspace external changes:")
    let firstPath: string | null = null
    let firstProviderRun: string | null = null
    for (const issue of externalChanges.issues) {
      const providerRun = issue.provider_run_id ?? "-"
      const root = issue.workspace_root ?? "-"
      firstPath ??= issue.path
      firstProviderRun ??= issue.provider_run_id ?? null
      lines.push(`  run=${providerRun} root=${root} path=${issue.path} fingerprint=${issue.workspace_fingerprint}`)
    }
    const pathTarget = firstPath ? `path ${firstPath}` : "the affected path"
    const runTarget = firstProviderRun ? ` for provider run ${firstProviderRun}` : ""
    lines.push(`  next: inspect ${pathTarget}${runTarget}; rerun or reconcile the affected managed/tracked turn before retrying workspace live sync`)
  }

  if (!liveSyncManagedMode.write_fence_supported && liveSyncManagedMode.unavailable_reason) {
    lines.push(`workspace live sync managed capability: unavailable (${liveSyncManagedMode.unavailable_reason}); tracked/off modes unaffected`)
  }

  if (externalChanges.live_watcher_scan_errors > 0) {
    lines.push(`workspace watcher scan errors: ${externalChanges.live_watcher_scan_errors}`)
    lines.push("  next: run /workspace sync status, then /workspace sync ignore; check .arrobaignore, selected workspace paths, and permissions before refreshing")
  }
}

function sliceStorageRecoveryAction(lastError?: string | null): string {
  const normalized = lastError?.toLowerCase() ?? ""
  if (
    normalized.includes("no space left on device")
    || normalized.includes("slice storage preflight failed")
    || normalized.includes("needs more free space")
  ) {
    return "free Docker/Colima disk or delete unused slice containers/volumes"
  }
  return ""
}

function formatRemoteRuntimeInvariantSummary(health: DaemonHealthProjection): string {
  const providerRuns = health.provider_runs
  const providerRunActor = health.provider_run_actor
  const remoteExecution = health.remote_execution
  const sliceLifecycle = health.slice_lifecycle
  const remoteExtensionSync = health.remote_extension_sync
  const liveSync = health.workspace_live_sync
  const liveSyncAffectedRoots = workspaceLiveSyncAffectedRoots(health)
  const workerRuns = remoteExecution.missing_active_worker_runs === 0 && remoteExecution.malformed_bindings === 0
    ? "ok"
    : `attention missing_worker_runs=${remoteExecution.missing_active_worker_runs} malformed=${remoteExecution.malformed_bindings}`
  const slices = sliceLifecycle.unhealthy_slices === 0
    && sliceLifecycle.failed_operations === 0
    && sliceLifecycle.starting_slices === 0
    && sliceLifecycle.stopping_slices === 0
    && sliceLifecycle.in_progress_operations === 0
    && sliceLifecycle.provider_auth_missing_slices === 0
    && sliceLifecycle.provider_auth_unconfigured_slices === 0
    ? "ok"
    : `attention starting=${sliceLifecycle.starting_slices} stopping=${sliceLifecycle.stopping_slices} in_progress=${sliceLifecycle.in_progress_operations} unhealthy=${sliceLifecycle.unhealthy_slices} failed_ops=${sliceLifecycle.failed_operations} auth_missing=${sliceLifecycle.provider_auth_missing_slices} auth_unconfigured=${sliceLifecycle.provider_auth_unconfigured_slices}`
  const manifests = remoteExtensionSync.manifest_missing_agents === 0
    && remoteExtensionSync.failed_agents === 0
    && remoteExtensionSync.pending_revoke_agents === 0
    && remoteExtensionSync.stale_agents === 0
    && remoteExtensionSync.pending_agents === 0
    && remoteExtensionSync.syncing_agents === 0
    ? "settled"
    : `attention syncing=${remoteExtensionSync.syncing_agents} pending=${remoteExtensionSync.pending_agents} failed=${remoteExtensionSync.failed_agents} stale=${remoteExtensionSync.stale_agents} missing=${remoteExtensionSync.manifest_missing_agents} pending_revoke=${remoteExtensionSync.pending_revoke_agents}`
  const liveSyncScope = liveSync.workspace_identity.identity_changed_provider_runs === 0
    && liveSync.workspace_identity.invalid_provider_runs === 0
    && liveSync.external_changes.externally_changed_artifacts === 0
    && liveSync.external_changes.live_watcher_scan_errors === 0
    ? "selected-workspace-only"
    : liveSyncScopeAttentionDetail(liveSyncAffectedRoots, liveSync.workspace_identity.identity_changed_provider_runs, liveSync.workspace_identity.invalid_provider_runs, liveSync.external_changes.externally_changed_artifacts, liveSync.external_changes.live_watcher_scan_errors)
  const providerRunIssues = providerRunInvariantIssueCount(health)
  const providerRunsSummary = providerRunIssues === 0 && providerRunActor.enqueue_rejections === 0
    ? "ok"
    : `attention duplicate=${duplicateProviderRunBindingCount(health)} mixed=${providerRuns.multi_interface_agent_bindings.length} orphaned=${providerRuns.orphaned_active_runs.length} pointer=${providerRuns.session_active_run_mismatches.length} terminal=${providerRuns.terminal_diagnostics.length} actor_rejects=${providerRunActor.enqueue_rejections}`
  return `remote runtime invariants: provider_runs=${providerRunsSummary}; worker_runs=${workerRuns}; slices=${slices}; manifests=${manifests}; live_sync_scope=${liveSyncScope}`
}

function workspaceLiveSyncScopeDetail(affectedRoots: readonly string[]): string {
  const base = "selected workspace/worktree only; other repositories unrestricted"
  if (affectedRoots.length === 0) {
    return base
  }
  return `${base}; affected roots: ${formatAffectedRoots(affectedRoots)}`
}

function liveSyncScopeAttentionDetail(
  affectedRoots: readonly string[],
  identityChanged: number,
  invalid: number,
  externalChanges: number,
  watcherScanErrors: number,
): string {
  const counts = `attention identity_changed=${identityChanged} invalid=${invalid} external_changes=${externalChanges} scan_errors=${watcherScanErrors}`
  const roots = formatAffectedRoots(affectedRoots)
  return roots ? `${counts} roots=${roots}` : counts
}

function workspaceLiveSyncAffectedRoots(health: DaemonHealthProjection): string[] {
  return uniqueNonEmpty([
    ...health.workspace_live_sync.workspace_identity.issues.map((issue) => issue.root),
    ...health.workspace_live_sync.external_changes.issues.map((issue) => issue.workspace_root),
  ].filter(nonEmptyRoot))
}

function nonEmptyRoot(root: string | null | undefined): root is string {
  return Boolean(root && root !== "-")
}

function formatAffectedRoots(affectedRoots: readonly string[]): string {
  if (affectedRoots.length === 0) {
    return ""
  }
  const shown = affectedRoots.slice(0, 3).join(", ")
  const more = affectedRoots.length > 3 ? ` +${affectedRoots.length - 3} more` : ""
  return `${shown}${more}`
}

function uniqueNonEmpty(values: readonly string[]): string[] {
  const unique: string[] = []
  for (const value of values) {
    const trimmed = value.trim()
    if (trimmed && !unique.includes(trimmed)) {
      unique.push(trimmed)
    }
  }
  return unique
}

function appendRemoteRuntimeProjectionInvariantIssues(lines: string[], health: DaemonHealthProjection): void {
  lines.push(`session projection invariant issues: mismatches=${health.projection_invariants.mismatches.length}`)
  for (const mismatch of health.projection_invariants.mismatches) {
    lines.push(`  ${mismatch.kind} session=${mismatch.session_id} agent=${mismatch.agent_id ?? "-"}: ${mismatch.details}`)
  }
  const first = health.projection_invariants.mismatches[0]
  const target = first?.agent_id
    ? `agent ${first.agent_id} in session ${first.session_id}`
    : first?.session_id
      ? `session ${first.session_id}`
      : "the affected session"
  lines.push(`  next: refresh ${target}; run /kernel health and /agent list; capture a debug bundle before restarting the kernel if the mismatch persists`)
}

function appendRemoteRuntimeProviderRunIssues(lines: string[], health: DaemonHealthProjection): void {
  const providerRuns = health.provider_runs
  const actorRejects = health.provider_run_actor.enqueue_rejections
  lines.push(`provider run issues: duplicate=${duplicateProviderRunBindingCount(health)} mixed=${providerRuns.multi_interface_agent_bindings.length} orphaned=${providerRuns.orphaned_active_runs.length} pointer=${providerRuns.session_active_run_mismatches.length} terminal=${providerRuns.terminal_diagnostics.length} actor_rejects=${actorRejects}`)
  for (const conflict of providerRuns.duplicate_arroba_agent_bindings) {
    lines.push(`  duplicate_arroba session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
  }
  for (const conflict of providerRuns.duplicate_native_tui_agent_bindings) {
    lines.push(`  duplicate_native_tui session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
  }
  for (const conflict of providerRuns.multi_interface_agent_bindings) {
    lines.push(`  mixed session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
  }
  for (const issue of providerRuns.orphaned_active_runs) {
    lines.push(`  orphaned run=${issue.provider_run_id} session=${issue.session_id} agent=${issue.agent_id ?? "-"}: ${issue.details}`)
  }
  for (const issue of providerRuns.session_active_run_mismatches) {
    lines.push(`  pointer session=${issue.session_id} active=${issue.active_provider_run_id ?? "-"}: ${issue.details}`)
  }
  for (const issue of providerRuns.terminal_diagnostics) {
    lines.push(`  terminal run=${issue.provider_run_id} provider=${issue.provider} state=${issue.state} session=${issue.session_id} agent=${issue.agent_id ?? "-"}: ${issue.diagnostic}`)
  }
  if (actorRejects > 0) {
    lines.push(`  actor rejected ${actorRejects} command${actorRejects === 1 ? "" : "s"}`)
  }
  lines.push(`  next: ${remoteRuntimeProviderRunNextAction(health)}`)
}

function remoteRuntimeProviderRunNextAction(health: DaemonHealthProjection): string {
  const providerRuns = health.provider_runs
  const duplicate = providerRuns.duplicate_arroba_agent_bindings[0]
  if (duplicate) {
    return duplicate.agent_id
      ? `run /agent inspect ${duplicate.agent_id}; run /provider processes; capture a debug bundle, then stop duplicate provider runs before sending prompts to that agent`
      : "run /provider processes; capture a debug bundle, then identify and stop duplicate provider runs before sending more prompts"
  }
  const duplicateNative = providerRuns.duplicate_native_tui_agent_bindings[0]
  if (duplicateNative) {
    return duplicateNative.agent_id
      ? `run /agent inspect ${duplicateNative.agent_id}; run /provider processes; close duplicate native TUIs before sending prompts to that agent`
      : "run /provider processes; identify and close duplicate native TUIs before sending more prompts"
  }
  const mixed = providerRuns.multi_interface_agent_bindings[0]
  if (mixed) {
    return mixed.agent_id
      ? `run /agent inspect ${mixed.agent_id}; run /provider processes; close the extra native TUI or Arroba provider run before sending prompts to that agent`
      : "run /provider processes; close the extra native TUI or Arroba provider run after identifying the affected agent"
  }
  const orphaned = providerRuns.orphaned_active_runs[0]
  if (orphaned) {
    const target = orphaned.provider_run_id ? `provider run ${orphaned.provider_run_id}` : "the orphaned provider run"
    return `refresh the session; stop or relaunch ${target} if it stays active`
  }
  const pointer = providerRuns.session_active_run_mismatches[0]
  if (pointer) {
    const target = pointer.session_id ? `session ${pointer.session_id}` : "the affected session"
    return `inspect ${target} and relaunch the affected agent to restore one active run pointer`
  }
  const terminal = providerRuns.terminal_diagnostics[0]
  if (terminal) {
    const target = terminal.provider_run_id ? `provider run ${terminal.provider_run_id}` : "the affected provider run"
    return terminal.agent_id
      ? `run /agent inspect ${terminal.agent_id}; run /provider processes; relaunch ${target} if the diagnostic persists`
      : `identify the affected agent from /provider processes or the debug bundle; run /provider processes; relaunch ${target} if the diagnostic persists`
  }
  return "wait for provider-run command queues to drain; inspect duplicate/stuck provider runs if rejections continue"
}

function providerRunRuntimeIssueCount(health: DaemonHealthProjection): number {
  return providerRunInvariantIssueCount(health) + providerRunActorHealthIssueCount(health)
}

function providerRunInvariantIssueCount(health: DaemonHealthProjection): number {
  return duplicateProviderRunBindingCount(health)
    + health.provider_runs.multi_interface_agent_bindings.length
    + health.provider_runs.orphaned_active_runs.length
    + health.provider_runs.session_active_run_mismatches.length
    + health.provider_runs.terminal_diagnostics.length
}

function duplicateProviderRunBindingCount(health: DaemonHealthProjection): number {
  return health.provider_runs.duplicate_arroba_agent_bindings.length
    + health.provider_runs.duplicate_native_tui_agent_bindings.length
}

function providerCatalogHealthIssueCount(health: DaemonHealthProjection): number {
  return health.provider_catalog.expired ? 1 : 0
}

function workspaceHealthIssueCount(health: DaemonHealthProjection): number {
  const externalChanges = health.workspace_live_sync.external_changes
  return health.workspace_coordination.worktree_collisions.length
    + health.workspace_live_sync.workspace_identity.identity_changed_provider_runs
    + health.workspace_live_sync.workspace_identity.invalid_provider_runs
    + (externalChanges.issues.length > 0
      ? externalChanges.issues.length
      : externalChanges.externally_changed_artifacts)
    + externalChanges.live_watcher_scan_errors
}

function workspaceRemoteRuntimeHardIssueCount(health: DaemonHealthProjection): number {
  const externalChanges = health.workspace_live_sync.external_changes
  return health.workspace_coordination.worktree_collisions.length
    + health.workspace_live_sync.workspace_identity.identity_changed_provider_runs
    + health.workspace_live_sync.workspace_identity.invalid_provider_runs
    + (externalChanges.issues.length > 0
      ? externalChanges.issues.length
      : externalChanges.externally_changed_artifacts)
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

function remoteExtensionSyncAggregateStatus(remoteExtensionSync: DaemonHealthProjection["remote_extension_sync"]): {
  readonly state: string
  readonly pending_revoke: boolean
} | null {
  if (remoteExtensionSync.pending_revoke_agents > 0) {
    return {
      state: remoteExtensionSync.failed_agents > 0
        ? "failed"
        : remoteExtensionSync.stale_agents > 0
          ? "stale"
          : remoteExtensionSync.manifest_missing_agents > 0
            ? "missing"
            : "pending",
      pending_revoke: true,
    }
  }
  if (remoteExtensionSync.manifest_missing_agents > 0) {
    return { state: "missing", pending_revoke: false }
  }
  if (remoteExtensionSync.failed_agents > 0) {
    return { state: "failed", pending_revoke: false }
  }
  if (remoteExtensionSync.stale_agents > 0) {
    return { state: "stale", pending_revoke: false }
  }
  return null
}

function remoteExtensionAggregateNextAction(remoteExtensionSync: DaemonHealthProjection["remote_extension_sync"]): string {
  if (remoteExtensionSync.pending_revoke_agents > 0) {
    return "keep the home revoke in place; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after the worker reconnects"
  }
  return "home keeps stale home-proxy calls blocked; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after worker connectivity is healthy"
}

function remoteExtensionSyncHardIssueCount(health: DaemonHealthProjection): number {
  if (health.remote_extension_sync.issues.length > 0) {
    return health.remote_extension_sync.issues.filter((issue) => (
      issue.pending_revoke
      || issue.state === "failed"
      || issue.state === "missing"
      || issue.state === "manifest_missing"
    )).length
  }
  return health.remote_extension_sync.failed_agents
    + health.remote_extension_sync.manifest_missing_agents
    + health.remote_extension_sync.pending_revoke_agents
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

function commandLaneInspectionTargets(lanes: readonly CommandLaneIssue[]): string {
  const targets = lanes.slice(0, 4).map(commandLaneInspectionTarget)
  if (lanes.length > 4) {
    targets.push(`${lanes.length - 4} more lane${lanes.length - 4 === 1 ? "" : "s"}`)
  }
  return targets.length > 0 ? targets.join(", ") : "stuck sessions/agents"
}

function commandLaneInspectionTarget(lane: CommandLaneIssue): string {
  switch (lane.kind) {
    case "session":
      return `session ${lane.laneId}`
    case "agent":
      return `agent ${lane.laneId}`
    case "workflow":
      return `workflow ${lane.laneId}`
    case "provider":
      return `provider run ${lane.laneId}`
  }
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
