import path from "node:path"

export const ARROBA_ASCII_ART = [
  "    _    ____  ____   ___  ____    _    ",
  "   / \\  |  _ \\|  _ \\ / _ \\| __ )  / \\   ",
  "  / _ \\ | |_) | |_) | | | |  _ \\ / _ \\  ",
  " / ___ \\|  _ <|  _ <| |_| | |_) / ___ \\ ",
  "/_/   \\_\\_| \\_\\_| \\_\\___/|____/_/   \\_\\",
].join("\n")

export const SESSION_NEW_HELP_TEXT = "Use `/session new [alias]` to start a session or `/session attach <ref>` to reattach."
export const SESSION_NEW_PLACEHOLDER = "Use /session new [alias] or /session attach <ref>"
export const SESSION_NEW_FOOTER_HINT = "No session • /session new or /session attach"
export const SESSION_NEW_ERROR_HINT = "No session attached. Use /session new or /session attach."

export type SessionListEntry = {
  id: string
  alias?: string | null
  worktree_id: string
  status: string
  attachment_ids: string[]
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
      const attachments = `${session.attachment_ids.length} ${session.attachment_ids.length === 1 ? "CLI" : "CLIs"}`
      const current = session.id === currentSessionId ? " current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${current}`
    }),
  ].join("\n")
}
