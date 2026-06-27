import type {
  RecallEvent,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./kernel-types.js"

export function formatWorkspaceLinks(links: WorkspaceLinkDefinition[]): string {
  if (links.length === 0) {
    return "no workspace links in session"
  }
  return links.map((link) => (
    `${link.name} (${link.link_id}) attachments=${link.attachments?.length ?? 0}`
  )).join("\n")
}

export function formatWorkspaceLinkDetails(link: WorkspaceLinkDefinition): string {
  const lines = [
    `workspace link ${link.name} (${link.link_id})`,
    `created_by=${link.created_by_user_id}`,
    `attachments=${link.attachments?.length ?? 0}`,
  ]
  for (const attachment of link.attachments ?? []) {
    const branch = attachment.branch ? ` branch=${attachment.branch}` : ""
    lines.push(`- ${attachment.user_id} ${attachment.repo_root}${branch}`)
  }
  return lines.join("\n")
}

export function formatWorkspaceLiveSyncStatus(status: WorkspaceLiveSyncStatus): string {
  const next = workspaceLiveSyncNextAction(status)
  const lines = [
    `workspace live sync: ${status.mode} footer=${status.footer_state}`,
    "scope=selected workspace/worktree only; other repositories are unrestricted",
    `sync_groups=${status.sync_groups.length}`,
    `targets=${status.targets.length} conflicts=${status.conflicts.length}`,
    `ignore=${status.ignore.ignore_file ?? "none"}`,
    next ? `next=${next}` : "",
  ]
  for (const group of status.sync_groups) {
    lines.push(`group ${group.group_name} (${group.group_id}) targets=${group.target_count} ready=${group.ready_targets} degraded=${group.degraded_targets} conflicts=${group.conflicted_targets}`)
  }
  for (const rule of status.ignore.rules) {
    lines.push(`rule ${rule}`)
  }
  for (const rule of status.ignore.force_excludes) {
    lines.push(`force-exclude ${rule}`)
  }
  for (const target of status.targets) {
    const branch = target.branch ? ` branch=${target.branch}` : ""
    const runtime = ` machine=${target.machine_id || "-"} kernel=${target.kernel_id || "-"}`
    lines.push(`- ${target.status} ${target.link_name}: ${target.user_id} ${target.repo_root}${branch}${runtime}`)
  }
  for (const conflict of status.conflicts) {
    lines.push(`! ${conflict.path} source=${conflict.source_agent_id} target=${conflict.target_user_id}:${conflict.target_repo_root} next=${conflict.next_action}`)
  }
  return lines.join("\n")
}

export function formatWorkspaceLiveSyncTargets(status: WorkspaceLiveSyncStatus): string {
  const lines = status.sync_groups.map((group) => (
    `group ${group.group_name} (${group.group_id}) targets=${group.target_count} ready=${group.ready_targets} degraded=${group.degraded_targets} conflicts=${group.conflicted_targets}`
  ))
  for (const target of status.targets) {
    const branch = target.branch ? ` branch=${target.branch}` : ""
    const runtime = ` machine=${target.machine_id || "-"} kernel=${target.kernel_id || "-"}`
    lines.push(`${target.status} ${target.link_name}: ${target.user_id} ${target.repo_root}${branch}${runtime}`)
  }
  if (lines.length === 0) {
    return "no workspace live sync targets\nnext=link another repo or worktree with workspace sync link <name-or-id> [repo-root]"
  }
  const next = workspaceLiveSyncNextAction(status)
  return next ? `${lines.join("\n")}\nnext=${next}` : lines.join("\n")
}

export function formatWorkspaceLiveSyncDoctor(status: WorkspaceLiveSyncStatus): string {
  const health = workspaceLiveSyncHealthLabel(status)
  const next = workspaceLiveSyncNextAction(status)
  const lines = [
    `workspace live sync doctor: ${health}`,
    `mode=${status.mode} footer=${status.footer_state}`,
    "scope=selected workspace/worktree only; other repositories are unrestricted",
    `groups=${status.sync_groups.length}`,
    `targets=${status.targets.length}`,
    `ready_targets=${status.sync_groups.reduce((sum, group) => sum + group.ready_targets, 0)}`,
    `degraded_targets=${status.sync_groups.reduce((sum, group) => sum + group.degraded_targets, 0)}`,
    `conflicted_targets=${status.sync_groups.reduce((sum, group) => sum + group.conflicted_targets, 0)}`,
    `conflicts=${status.conflicts.length}`,
  ]
  const problems = workspaceLiveSyncProblems(status)
  if (problems.length > 0) {
    lines.push("problems:")
    lines.push(...problems.map((problem) => `- ${problem}`))
  } else {
    lines.push("problems=none")
  }
  if (next) lines.push(`next=${next}`)
  lines.push("inspect=workspace sync targets; workspace sync conflicts; workspace sync ignore; workspace sync audit")
  return lines.join("\n")
}

export function formatWorkspaceLiveSyncAudit(events: readonly RecallEvent[]): string {
  if (events.length === 0) {
    return "no workspace live sync audit events\nnext=change mode with workspace sync off|managed|tracked, then rerun workspace sync audit"
  }
  return [
    `workspace live sync audit: ${events.length}`,
    ...events.map(formatWorkspaceLiveSyncAuditEvent),
    "next=use workspace sync status for current health, workspace sync conflicts for unresolved fanout conflicts",
  ].join("\n")
}

export function workspaceLiveSyncNextAction(status: WorkspaceLiveSyncStatus): string {
  if (status.conflicts.length > 0 || status.footer_state === "conflict") {
    return "inspect workspace sync conflicts, ask an agent to reconcile, then rerun workspace sync status"
  }
  if (status.mode === "unrestricted" || status.footer_state === "off") {
    return "enable with workspace sync tracked for turn-end fanout, or use workspace sync managed on hosts with managed write fencing"
  }
  if (status.targets.length === 0) {
    return "link another repo or worktree with workspace sync link <name-or-id> [repo-root]"
  }
  if (status.sync_groups.some((group) => group.degraded_targets > 0) || status.targets.some((target) => target.status === "degraded")) {
    return "inspect workspace sync targets and reconnect or repair degraded target kernels"
  }
  if (status.footer_state === "syncing") {
    return "wait for sync to settle, or inspect workspace sync targets"
  }
  return ""
}

function formatWorkspaceLiveSyncAuditEvent(event: RecallEvent): string {
  const metadata = event.metadata ?? {}
  const timestamp = Number.isFinite(event.timestamp_ms)
    ? new Date(event.timestamp_ms).toISOString()
    : "unknown-time"
  const previousMode = metadataString(metadata.previous_mode) ?? "config-default"
  const mode = metadataString(metadata.mode) ?? "unknown"
  const caller = metadataString(metadata.caller_user_id) ?? "unknown-user"
  const source = metadataString(metadata.command_source) ?? "unknown-source"
  const callerKind = metadataString(metadata.caller_kind)
  const client = metadataString(metadata.client_id)
  const machine = metadataString(metadata.machine_id)
  const scope = metadataString(metadata.scope) ?? "selected_workspace_worktree"
  const otherRepos = metadataString(metadata.other_repositories) ?? "unrestricted"
  return [
    `- ${timestamp} ${previousMode} -> ${mode} by ${caller} via ${source}`,
    `  scope=${scope}; other_repositories=${otherRepos}`,
    [
      callerKind ? `caller=${callerKind}` : null,
      client ? `client=${client}` : null,
      machine ? `machine=${machine}` : null,
      event.worktree_path ? `worktree=${event.worktree_path}` : null,
    ].filter(Boolean).join(" "),
  ].filter(Boolean).join("\n")
}

function metadataString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null
}

function workspaceLiveSyncHealthLabel(status: WorkspaceLiveSyncStatus): string {
  if (status.conflicts.length > 0 || status.footer_state === "conflict") return "conflict"
  if (status.sync_groups.some((group) => group.degraded_targets > 0) || status.targets.some((target) => target.status === "degraded")) return "degraded"
  if (status.mode === "unrestricted" || status.footer_state === "off") return "off"
  if (status.targets.length === 0) return "no-targets"
  if (status.footer_state === "syncing") return "syncing"
  return "healthy"
}

function workspaceLiveSyncProblems(status: WorkspaceLiveSyncStatus): string[] {
  const problems: string[] = []
  if (status.mode === "unrestricted" || status.footer_state === "off") {
    problems.push("live sync is off for this session")
  }
  if (status.targets.length === 0 && status.mode !== "unrestricted") {
    problems.push("no synced worktrees or remote attachments are linked")
  }
  for (const group of status.sync_groups) {
    if (group.degraded_targets > 0) {
      problems.push(`${group.group_name} has ${group.degraded_targets} degraded target${group.degraded_targets === 1 ? "" : "s"}`)
    }
    if (group.conflicted_targets > 0) {
      problems.push(`${group.group_name} has ${group.conflicted_targets} conflicted target${group.conflicted_targets === 1 ? "" : "s"}`)
    }
  }
  for (const conflict of status.conflicts) {
    problems.push(`${conflict.path} from ${conflict.source_agent_id} blocked on ${conflict.target_user_id}:${conflict.target_repo_root}`)
  }
  return problems
}
