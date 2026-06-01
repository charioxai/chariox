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
}

export function formatKernelHealth(health: DaemonHealthProjection): string {
  const providerRuns = health.provider_runs
  const lines = [
    "kernel health",
    `provider runs: projected=${providerRuns.projected_runs} active=${providerRuns.active_runs} arroba=${providerRuns.arroba_active_runs} native_tui=${providerRuns.native_tui_active_runs}`,
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
