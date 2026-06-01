import type { RuntimeSession } from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { DaemonHealthProjection } from "@arroba/kernel-client"

type FooterTone = "info" | "error"

export type KernelCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  flashFooter: (message: string, tone: FooterTone) => void
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  getDaemonHealth?: () => Promise<DaemonHealthProjection>
  appendNotice: (message: string) => void
  transitionToNoSession: (message: string) => void
}

export async function handleKernelSlashCommand(
  deps: KernelCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "kernel" }>,
): Promise<void> {
  const [subcommand, ...args] = command.args
  if (subcommand === "health" || subcommand === "status") {
    if (!deps.getDaemonHealth) {
      deps.flashFooter("kernel health is unavailable in this build", "error")
      return
    }
    if (args.length > 0) {
      deps.flashFooter("usage: /kernel health", "error")
      return
    }
    const health = await deps.getDaemonHealth()
    const issueCount = kernelHealthIssueCount(health)
    deps.appendNotice(formatKernelHealth(health))
    deps.flashFooter(
      issueCount === 0
        ? "kernel health: ok"
        : `kernel health: ${issueCount} issue${issueCount === 1 ? "" : "s"}`,
      issueCount === 0 ? "info" : "error",
    )
    return
  }
  if (subcommand === "delete") {
    if (!deps.deleteKernel) {
      deps.flashFooter("kernel delete is unavailable in this build", "error")
      return
    }
    if (args.length > 0) {
      deps.flashFooter("usage: /kernel delete", "error")
      return
    }
    const deleted = await deps.deleteKernel()
    if (deps.isAttached() && deleted.deletedSessions.some((session) => session.id === deps.sessionState().id)) {
      deps.transitionToNoSession(`Kernel ${deleted.kernelId} was deleted.`)
      return
    }
    deps.flashFooter(`deleted kernel ${deleted.kernelId} (${deleted.deletedSessions.length} session${deleted.deletedSessions.length === 1 ? "" : "s"})`, "info")
    return
  }
  deps.flashFooter("usage: /kernel health | /kernel delete", "error")
}

export function kernelHealthIssueCount(health: DaemonHealthProjection): number {
  return health.provider_runs.duplicate_arroba_agent_bindings.length
    + health.provider_runs.orphaned_active_runs.length
    + health.provider_runs.session_active_run_mismatches.length
    + health.projection_invariants.mismatches.length
    + workspaceHealthIssueCount(health)
    + transportHealthIssueCount(health)
    + terminalStreamHealthIssueCount(health)
    + capabilityHealthIssueCount(health)
}

export function formatKernelHealth(health: DaemonHealthProjection): string {
  const providerRuns = health.provider_runs
  const capability = health.capability_executor
  const transport = health.transport
  const terminalStream = health.terminal_stream
  const workspaceCoordination = health.workspace_coordination
  const liveSync = health.workspace_live_sync
  const workspaceIdentity = liveSync.workspace_identity
  const externalChanges = liveSync.external_changes
  const lines = [
    "kernel health",
    `provider runs: projected=${providerRuns.projected_runs} active=${providerRuns.active_runs} arroba=${providerRuns.arroba_active_runs} native_tui=${providerRuns.native_tui_active_runs}`,
    `capabilities: running=${capability.running_jobs}/${capability.max_concurrent_jobs} submitted=${capability.submitted_jobs} failed=${capability.failed_jobs} rejected=${capability.rejected_jobs} join_errors=${capability.join_errors}`,
    `transport: connections=${transport.active_connections} subscriptions=${transport.active_subscriptions} incoming=${transport.incoming_requests} emitted=${transport.emitted_events} replay_gaps=${transport.replay_gaps} overloads=${transport.inbound_overload_rejections} duplicate_commands=${transport.duplicate_command_conflicts} outgoing_overflows=${transport.outgoing_queue_overflows} slow_consumers=${transport.slow_consumer_closes}`,
    `terminal stream: pending_output=${terminalStream.pending_output_records} pending_notices=${terminalStream.pending_notice_records} pending_completions=${terminalStream.pending_completion_records} trimmed_recipients=${terminalStream.trimmed_pending_output_recipients} limit=${terminalStream.pending_output_record_limit_per_attachment}`,
    `workspace coordination: claims=${workspaceCoordination.active_worktree_claims.length} collisions=${workspaceCoordination.worktree_collisions.length} active_ops=${workspaceCoordination.active_operation_claims.length}`,
    `workspace live sync: reservations=${liveSync.active_reservations} artifacts=${liveSync.active_reservation_artifacts} tracked_runs=${workspaceIdentity.tracked_provider_runs} identity_changed=${workspaceIdentity.identity_changed_provider_runs} invalid_runs=${workspaceIdentity.invalid_provider_runs}`,
    `workspace watcher: tracked=${externalChanges.tracked_artifacts} external_changes=${externalChanges.externally_changed_artifacts} events=${externalChanges.external_change_events} scans=${externalChanges.live_watcher_scans} scan_errors=${externalChanges.live_watcher_scan_errors} started=${externalChanges.live_watcher_started ? "yes" : "no"}`,
  ]

  if (providerRuns.duplicate_arroba_agent_bindings.length === 0) {
    lines.push("provider run bindings: ok")
  } else {
    lines.push("duplicate Arroba provider run bindings:")
    for (const conflict of providerRuns.duplicate_arroba_agent_bindings) {
      lines.push(`  session=${conflict.session_id} agent=${conflict.agent_id} runs=${conflict.provider_run_ids.join(",")}`)
    }
  }

  if (providerRuns.orphaned_active_runs.length > 0) {
    lines.push("orphaned active provider runs:")
    for (const issue of providerRuns.orphaned_active_runs) {
      lines.push(`  run=${issue.provider_run_id} session=${issue.session_id} agent=${issue.agent_id ?? "-"}: ${issue.details}`)
    }
  }

  if (providerRuns.session_active_run_mismatches.length > 0) {
    lines.push("session active provider run pointer issues:")
    for (const issue of providerRuns.session_active_run_mismatches) {
      lines.push(`  session=${issue.session_id} active=${issue.active_provider_run_id ?? "-"}: ${issue.details}`)
    }
  }

  if (capability.rejected_jobs > 0 || capability.join_errors > 0) {
    lines.push(`capability executor issues: rejected=${capability.rejected_jobs} join_errors=${capability.join_errors}`)
  }

  const transportIssueCount = transportHealthIssueCount(health)
  if (transportIssueCount > 0) {
    lines.push(`transport issues: replay_gaps=${transport.replay_gaps} overloads=${transport.inbound_overload_rejections} duplicate_commands=${transport.duplicate_command_conflicts} outgoing_overflows=${transport.outgoing_queue_overflows} slow_consumers=${transport.slow_consumer_closes}`)
  }

  if (terminalStream.trimmed_pending_output_recipients > 0) {
    lines.push(`terminal stream trimmed pending output for ${terminalStream.trimmed_pending_output_recipients} recipient${terminalStream.trimmed_pending_output_recipients === 1 ? "" : "s"}`)
  }

  if (workspaceCoordination.worktree_collisions.length > 0) {
    lines.push("workspace worktree collisions:")
    for (const collision of workspaceCoordination.worktree_collisions) {
      lines.push(`  workspace=${collision.workspace_id} worktree=${collision.worktree_id} sessions=${collision.session_ids.join(",")}`)
    }
  }

  if (workspaceIdentity.identity_changed_provider_runs > 0 || workspaceIdentity.invalid_provider_runs > 0) {
    lines.push(`workspace identity issues: changed=${workspaceIdentity.identity_changed_provider_runs} invalid=${workspaceIdentity.invalid_provider_runs}`)
  }

  if (externalChanges.live_watcher_scan_errors > 0) {
    lines.push(`workspace watcher scan errors: ${externalChanges.live_watcher_scan_errors}`)
  }

  if (health.projection_invariants.mismatches.length === 0) {
    lines.push(`projection invariants: ok (${health.projection_invariants.checked_sessions} session${health.projection_invariants.checked_sessions === 1 ? "" : "s"}, ${health.projection_invariants.checked_agents} agent${health.projection_invariants.checked_agents === 1 ? "" : "s"})`)
  } else {
    lines.push("projection invariant mismatches:")
    for (const mismatch of health.projection_invariants.mismatches) {
      lines.push(`  ${mismatch.kind} session=${mismatch.session_id} agent=${mismatch.agent_id ?? "-"}: ${mismatch.details}`)
    }
  }

  return lines.join("\n")
}

function workspaceHealthIssueCount(health: DaemonHealthProjection): number {
  return health.workspace_coordination.worktree_collisions.length
    + health.workspace_live_sync.workspace_identity.identity_changed_provider_runs
    + health.workspace_live_sync.workspace_identity.invalid_provider_runs
    + health.workspace_live_sync.external_changes.live_watcher_scan_errors
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

function capabilityHealthIssueCount(health: DaemonHealthProjection): number {
  return health.capability_executor.rejected_jobs + health.capability_executor.join_errors
}
