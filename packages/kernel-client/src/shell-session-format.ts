import { basename } from "node:path"

import type {
  CloudCollaborator,
  CloudSessionMember,
  RuntimeSession,
  SessionInvite,
  SessionMember,
} from "./kernel-types.js"
import {
  remoteWorkerProviderRunIsMissing,
  remoteWorkerProviderRunRecoveryAction,
} from "./provider-run-recovery.js"
import { formatSessionHomeKernelLabel } from "./session-runtime-labels.js"
import { sessionAgentIsBusy } from "./shell-agent-activity.js"
import { formatWorkspaceLiveSyncModeLabel } from "./workspace-live-sync-mode.js"

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
      const runtime = formatSessionRuntimeSummary(session)
      const remote = formatSessionRemoteRuntime(session)
      const current = session.id === currentSessionId ? " current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${runtime}${remote}${current}`
    }),
  ].join("\n")
}

function formatSessionRuntimeSummary(session: RuntimeSession): string {
  const parts = [
    `home ${formatSessionHomeKernelLabel(session, "unknown")}`,
    session.owner_user_id?.trim() ? `owner ${session.owner_user_id.trim()}` : null,
    "authority home-owned",
    `live sync ${formatWorkspaceLiveSyncModeLabel(session.workspace_live_sync_mode)}`,
  ].filter(Boolean)
  return ` - ${parts.join(" - ")}`
}

function formatSessionRemoteRuntime(session: RuntimeSession): string {
  const remoteAgents = session.agents.filter((agent) => agent.remote_execution)
  if (remoteAgents.length === 0) return ""
  const workerRunGaps = remoteAgents.filter((agent) => remoteAgentHasWorkerRunGap(session, agent))
  const sliceAgents = remoteAgents.filter(remoteAgentIsSliceBacked)
  const remoteParts = [
    `${remoteAgents.length} agent${remoteAgents.length === 1 ? "" : "s"}`,
    sliceAgents.length > 0 ? `${sliceAgents.length} slice${sliceAgents.length === 1 ? "" : "s"}` : null,
  ].filter(Boolean)
  const remote = ` - remote ${remoteParts.join(", ")}`
  if (workerRunGaps.length === 0) return remote
  const target = workerRunGaps[0]
  const next = remoteWorkerProviderRunRecoveryAction(
    target?.agent_ref || target?.id,
    target?.remote_execution?.worker_machine_id,
  )
  return `${remote}, ${workerRunGaps.length} worker run gap${workerRunGaps.length === 1 ? "" : "s"} - next ${next}`
}

function remoteAgentHasWorkerRunGap(session: RuntimeSession, agent: RuntimeSession["agents"][number]): boolean {
  return remoteWorkerProviderRunIsMissing({
    agent,
    agentBusy: session.agent_activity || session.prompt_states
      ? sessionAgentIsBusy(session, agent.id)
      : null,
  })
}

function remoteAgentIsSliceBacked(agent: RuntimeSession["agents"][number]): boolean {
  return agent.remote_execution?.worker_kernel_id?.startsWith("slice:") ?? false
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
