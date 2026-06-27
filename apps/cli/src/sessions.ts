import path from "node:path"

import { remoteWorkerProviderRunRecoveryAction } from "@arroba/kernel-client/provider-run-recovery"

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
  alias?: string | null
  workspace_id?: string
  worktree_id: string
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  host_machine_id?: string | null
  host_daemon_id?: string | null
  kernel_id?: string | null
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  status: string
  created_at_ms?: number
  last_used_at_ms?: number | null
  attachment_ids?: string[]
  connected_cli_count?: number
  activity?: SessionActivitySummary
}

export type SessionActivitySummary = {
  agent_count: number
  working_agent_count: number
  active_prompt_count: number
  queued_prompt_count: number
  error_agent_count: number
  remote_agent_count?: number
  missing_worker_provider_run_count?: number
  home_proxy_agent_count?: number
  remote_extension_sync_issue_count?: number
  remote_extension_pending_revoke_count?: number
}

export type SessionBootstrapDecision =
  | { action: "create" }
  | { action: "resolve"; sessionRef: string }
  | { action: "attach_existing"; sessionId: string }
  | { action: "none" }

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
      const liveSync = formatSessionLiveSyncLabel(session)
      const remote = formatSessionRemoteActivity(session)
      const next = formatSessionActivityNext(session)
      const current = session.id === currentSessionId ? " - current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${home} - sync ${liveSync}${remote}${next}${current}`
    }),
  ].join("\n")
}

function formatSessionHomeKernel(session: SessionListEntry): string {
  const home = formatSessionHomeLabel(session)
  return home === "-" ? "" : ` - home ${home}`
}

export function formatSessionHomeLabel(session: Pick<SessionListEntry, "host_daemon_id" | "host_machine_id" | "kernel_id">): string {
  const kernel = session.host_daemon_id?.trim() || session.kernel_id?.trim()
  const machine = session.host_machine_id?.trim()
  return kernel && machine ? `${kernel}@${machine}` : kernel || machine || "-"
}

export function formatSessionLiveSyncLabel(session: Pick<SessionListEntry, "workspace_live_sync_mode">): string {
  const mode = session.workspace_live_sync_mode
  return mode === "managed" || mode === "tracked" ? mode : "off"
}

function formatSessionRemoteActivity(session: Pick<SessionListEntry, "activity">): string {
  const remoteAgents = session.activity?.remote_agent_count ?? 0
  const workerRunGaps = session.activity?.missing_worker_provider_run_count ?? 0
  const homeProxyAgents = session.activity?.home_proxy_agent_count ?? 0
  const remoteExtensionSyncIssues = session.activity?.remote_extension_sync_issue_count ?? 0
  const pendingRevokes = session.activity?.remote_extension_pending_revoke_count ?? 0
  const parts = [
    remoteAgents > 0 ? `${remoteAgents} remote ${remoteAgents === 1 ? "agent" : "agents"}` : "",
    workerRunGaps > 0 ? `${workerRunGaps} worker run ${workerRunGaps === 1 ? "gap" : "gaps"}` : "",
    homeProxyAgents > 0 ? `${homeProxyAgents} home-proxy ${homeProxyAgents === 1 ? "agent" : "agents"}` : "",
    remoteExtensionSyncIssues > 0 ? `${remoteExtensionSyncIssues} extension sync ${remoteExtensionSyncIssues === 1 ? "issue" : "issues"}` : "",
    pendingRevokes > 0 ? `${pendingRevokes} pending ${pendingRevokes === 1 ? "revoke" : "revokes"}` : "",
  ].filter(Boolean)
  return parts.length ? ` - ${parts.join(", ")}` : ""
}

function formatSessionActivityNext(session: Pick<SessionListEntry, "activity">): string {
  if ((session.activity?.missing_worker_provider_run_count ?? 0) > 0) {
    return ` - next: ${remoteWorkerProviderRunRecoveryAction(null, null)}`
  }
  const activity = session.activity
  if (activity && (activity.remote_extension_sync_issue_count ?? 0) > 0) {
    return ` - next: ${formatRemoteExtensionAggregateNextAction(activity)}`
  }
  return ""
}

function formatRemoteExtensionAggregateNextAction(activity: SessionActivitySummary): string {
  if ((activity.remote_extension_pending_revoke_count ?? 0) > 0) {
    return "keep the home revoke in place; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after the worker reconnects"
  }
  return "home keeps stale home-proxy calls blocked; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after worker connectivity is healthy"
}

export function formatSessionDisplayLabel(session: { id: string; alias?: string | null }) {
  return session.alias ?? session.id
}

export function sessionBrowserTitle(session: { id: string; alias?: string | null }) {
  return (session.alias ? `${session.id} (${session.alias})` : session.id).slice(0, 30)
}

export function sessionBrowserStatus(session: { status: string }) {
  return session.status.charAt(0).toUpperCase() + session.status.slice(1).toLowerCase()
}

export function sessionBrowserTimestamp(value: number | null) {
  if (value === null) {
    return "-"
  }
  const date = new Date(value)
  const year = date.getUTCFullYear()
  const month = String(date.getUTCMonth() + 1).padStart(2, "0")
  const day = String(date.getUTCDate()).padStart(2, "0")
  const hours = String(date.getUTCHours()).padStart(2, "0")
  const minutes = String(date.getUTCMinutes()).padStart(2, "0")
  return `${year}-${month}-${day} ${hours}:${minutes} UTC`
}

export function sessionBrowserSortTime(session: { last_used_at_ms?: number | null; created_at_ms?: number | null }) {
  return session.last_used_at_ms ?? session.created_at_ms ?? 0
}

export function selectAttachableSession(
  sessions: SessionListEntry[],
  workspace: string,
  worktree: string,
) {
  return sessions
    .filter((session) => session.workspace_id === workspace && session.worktree_id === worktree && session.status !== "Ended")
    .sort((left, right) => (right.created_at_ms ?? 0) - (left.created_at_ms ?? 0))[0] ?? null
}

export function decideBootstrapAction(
  options: { createSession?: boolean; sessionId?: string },
  sessions: SessionListEntry[],
  workspace: string,
  worktree: string,
): SessionBootstrapDecision {
  if (options.createSession) {
    return { action: "create" }
  }
  if (options.sessionId) {
    return { action: "resolve", sessionRef: options.sessionId }
  }
  const existing = selectAttachableSession(sessions, workspace, worktree)
  if (existing) {
    return { action: "none" }
  }
  return { action: "none" }
}
