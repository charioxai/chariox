import path from "node:path"

import { formatSessionHomeKernelLabel } from "@arroba/kernel-client/session-runtime-labels"
import { formatWorkspaceLiveSyncModeCompactLabel } from "@arroba/kernel-client/workspace-live-sync-mode"
import {
  waitingRoomSessionActivityNextAction,
  waitingRoomSessionRecencyMs,
  waitingRoomSessionRemoteActivityLabel,
  waitingRoomSessionStatusLabel,
  waitingRoomTimestampLabel,
} from "@arroba/kernel-client/waiting-room-activity"
import {
  decideBootstrapAction,
  selectAttachableSession,
  type SessionBootstrapDecision,
} from "@arroba/kernel-client/session-bootstrap-policy"

import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"

export const ARROBA_ASCII_ART = [
  "    _    ____  ____   ___  ____    _     _",
  "   / \\  |  _ \\ |  _ \\  / _ \\| __ )  / \\   ",
  "  / _ \\ | |_)   | |_) | | | | || _    / _ \\  ",
  " / ___ \\|  _ <  |  _ <| | |_| || _)  / ___ \\ ",
  "/_/   \\_\\_| \\_\\_| \\_\\___/|| _ ) /_/  \\_\\",
].join("\n")

export const SESSION_NEW_HELP_TEXT = "Use arrows to choose provider, model, variant, worktree, theme, session preview, or remote inventory. Enter on Join Existing Session opens all sessions. A archives sessions and D deletes selected sessions or inactive remote inventory."
export const SESSION_NEW_PLACEHOLDER = "Use the waiting room arrows to choose your next session"
export const SESSION_NEW_FOOTER_HINT = `Waiting room • arrows move • Enter confirms • A archives • D deletes inactive • ${HOTKEY_TOGGLE_LABEL} hotkeys`
export const SESSION_NEW_ERROR_HINT = "No session attached. Use the waiting room to create or join a session."

export type SessionListEntry = {
  id: string
  project_id?: string
  alias?: string | null
  workspace_id?: string
  worktree_id: string
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  host_machine_id?: string | null
  host_daemon_id?: string | null
  kernel_id?: string | null
  machine_id?: string | null
  kernel_alias?: string | null
  machine_alias?: string | null
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  status: string
  created_at_ms?: number
  last_used_at_ms?: number | null
  last_activity_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  attachment_ids?: string[]
  connected_cli_count?: number
  joined_collaborator_count?: number
  pending_collaboration_invite_count?: number
  activity?: SessionActivitySummary
  agents?: Array<{
    id: string
    alias?: string | null
    provider: string
    model?: string | null
    variant?: string | null
    mode?: string | null
    permission?: string | null
    activity?: {
      working?: boolean
      error?: boolean
      unread_idle_output?: boolean
    }
  }>
  workflows?: Array<{
    id: string
    alias?: string | null
    activity?: { working?: boolean; error?: boolean }
  }>
}

export type SessionActivitySummary = {
  agent_count: number
  working_agent_count: number
  active_prompt_count: number
  queued_prompt_count: number
  error_agent_count: number
  unread_idle_agent_count?: number
  remote_agent_count?: number
  missing_worker_provider_run_count?: number
  home_proxy_agent_count?: number
  remote_extension_sync_issue_count?: number
  remote_extension_pending_revoke_count?: number
}

export {
  decideBootstrapAction,
  selectAttachableSession,
  type SessionBootstrapDecision,
}

export function formatSessionList(sessions: SessionListEntry[], currentSessionId?: string) {
  if (sessions.length === 0) {
    return "No sessions found."
  }

  return [
    "Sessions",
    ...sessions.map((session) => {
      const name = session.alias ? `\`${session.alias}\` (\`${session.id}\`)` : `\`${session.id}\``
      const location = path.basename(session.worktree_id) || session.worktree_id
      const attachmentCount = session.attachment_ids?.length ?? session.connected_cli_count ?? 0
      const attachments = `${attachmentCount} ${attachmentCount === 1 ? "CLI" : "CLIs"}`
      const home = formatSessionHomeKernel(session)
      const liveSync = formatWorkspaceLiveSyncModeCompactLabel(session.workspace_live_sync_mode)
      const remote = formatSessionRemoteActivity(session)
      const next = formatSessionActivityNext(session)
      const current = session.id === currentSessionId ? " - current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${home} - sync ${liveSync}${remote}${next}${current}`
    }),
  ].join("\n")
}

function formatSessionHomeKernel(session: SessionListEntry): string {
  const home = formatSessionHomeKernelLabel(session)
  return home === "-" ? "" : ` - home ${home}`
}

function formatSessionRemoteActivity(session: Pick<SessionListEntry, "activity">): string {
  const label = waitingRoomSessionRemoteActivityLabel(session.activity)
  return label ? ` - ${label}` : ""
}

function formatSessionActivityNext(session: Pick<SessionListEntry, "activity">): string {
  const action = waitingRoomSessionActivityNextAction(session)
  return action ? ` - next: ${action}` : ""
}

export function formatSessionDisplayLabel(session: { id: string; alias?: string | null }) {
  return session.alias ?? session.id
}

export function sessionBrowserTitle(session: { id: string; alias?: string | null }) {
  return (session.alias ? `${session.id} (${session.alias})` : session.id).slice(0, 30)
}

export function sessionBrowserStatus(session: Pick<SessionListEntry, "status" | "activity">) {
  return waitingRoomSessionStatusLabel(session)
}

export function sessionBrowserTimestamp(value: number | null) {
  return waitingRoomTimestampLabel(value)
}

export function sessionBrowserSortTime(session: {
  last_prompt_sent_at_ms?: number | null
  last_activity_at_ms?: number | null
  last_used_at_ms?: number | null
  created_at_ms?: number | null
}) {
  return waitingRoomSessionRecencyMs(session)
}
