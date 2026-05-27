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
  const lines = [
    `workspace live sync: ${status.mode} footer=${status.footer_state}`,
    `targets=${status.targets.length} conflicts=${status.conflicts.length}`,
    `ignore=${status.ignore.ignore_file ?? "none"}`,
  ]
  for (const target of status.targets) {
    const branch = target.branch ? ` branch=${target.branch}` : ""
    lines.push(`- ${target.status} ${target.link_name}: ${target.user_id} ${target.repo_root}${branch}`)
  }
  for (const conflict of status.conflicts) {
    lines.push(`! ${conflict.path} source=${conflict.source_agent_id} target=${conflict.target_user_id}:${conflict.target_repo_root} next=${conflict.next_action}`)
  }
  return lines.join("\n")
}
