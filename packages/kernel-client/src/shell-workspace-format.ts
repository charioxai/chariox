import type {
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
