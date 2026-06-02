import { basename } from "node:path"

import type {
  AgentInstance,
  CloudCollaborator,
  CloudSessionMember,
  RuntimeSession,
  SessionInvite,
  SessionMember,
} from "./kernel-types.js"

export function formatSessionList(sessions: RuntimeSession[], currentSessionId?: string): string {
  if (sessions.length === 0) {
    return "No sessions found."
  }
  return [
    "Sessions",
    ...sessions.map((session) => {
      const name = session.alias ? `\`${session.alias}\` (\`${session.id}\`)` : `\`${session.id}\``
      const location = basename(session.worktree_id) || session.worktree_id
      const attachments = `${session.attachment_ids.length} ${session.attachment_ids.length === 1 ? "CLI" : "CLIs"}`
      const home = formatSessionHomeKernel(session)
      const remote = formatSessionRemoteRuntime(session)
      const current = session.id === currentSessionId ? " current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${home}${remote}${current}`
    }),
  ].join("\n")
}

function formatSessionHomeKernel(session: RuntimeSession): string {
  const host = session.host_daemon_id?.trim() || session.host_machine_id?.trim()
  return host ? ` - home ${host}` : ""
}

function formatSessionRemoteRuntime(session: RuntimeSession): string {
  const remoteAgents = session.agents.filter((agent) => agent.remote_execution)
  if (remoteAgents.length === 0) return ""
  const workerRunGaps = remoteAgents.filter(remoteAgentHasWorkerRunGap)
  const remote = ` - remote ${remoteAgents.length} agent${remoteAgents.length === 1 ? "" : "s"}`
  if (workerRunGaps.length === 0) return remote
  const target = workerRunGaps[0]?.agent_ref || workerRunGaps[0]?.id || "<agent>"
  return `${remote}, ${workerRunGaps.length} worker run gap${workerRunGaps.length === 1 ? "" : "s"} - next run /agent inspect ${target}; /kernel health`
}

function remoteAgentHasWorkerRunGap(agent: AgentInstance): boolean {
  const remote = agent.remote_execution
  if (!remote) return false
  const workerRun = remote.active_worker_provider_run_id?.trim()
  if (workerRun) return false
  return agent.state === "Working" || agent.is_processing
}

export function formatSessionMembers(members: SessionMember[], invites: SessionInvite[]): string {
  const lines = ["Session members"]
  if (members.length === 0) {
    lines.push("- none")
  } else {
    for (const member of members) {
      const inviter = member.invited_by_user_id ? ` invited_by=${member.invited_by_user_id}` : ""
      lines.push(`- ${member.user_id}${inviter}`)
    }
  }
  lines.push("Session invites")
  const activeInvites = invites.filter((invite) => !invite.revoked_at_ms)
  if (activeInvites.length === 0) {
    lines.push("- none")
  } else {
    for (const invite of activeInvites) {
      const maxUses = invite.max_uses ?? "unlimited"
      lines.push(`- ${invite.invite_id} uses=${invite.used_count}/${maxUses}`)
    }
  }
  return lines.join("\n")
}

export function formatSessionInvite(invite: SessionInvite, inviteToken: string): string {
  const maxUses = invite.max_uses ?? "unlimited"
  const expires = invite.expires_at_ms ? ` expires_at=${invite.expires_at_ms}` : ""
  return `session invite ${invite.invite_id} uses=0/${maxUses}${expires}\n${inviteToken}`
}

export function formatCloudMembers(members: CloudSessionMember[]): string {
  if (members.length === 0) {
    return "no cloud members in session"
  }
  return members.map((member) => (
    `${member.user_id} ${member.email}${member.display_name ? ` (${member.display_name})` : ""}`
  )).join("\n")
}

export function formatCloudCollaborators(collaborators: CloudCollaborator[]): string {
  if (collaborators.length === 0) {
    return "no recent cloud collaborators"
  }
  return collaborators.map((collaborator) => (
    `${collaborator.user_id} ${collaborator.email} shared_sessions=${collaborator.shared_session_count}`
  )).join("\n")
}
