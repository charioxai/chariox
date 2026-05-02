import path from "node:path"

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
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  status: string
  created_at_ms?: number
  last_used_at_ms?: number | null
  attachment_ids?: string[]
  connected_cli_count?: number
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
      const current = session.id === currentSessionId ? " current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${current}`
    }),
  ].join("\n")
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
