import {
  parseSlashCommand,
} from "./commands.js"

export type WaitingRoomSlashCommandPolicyDeps = {
  clearCommandCenter: () => void
  clearPromptText: () => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleWaitingRoomSlashCommand(
  rawPrompt: string,
  deps: WaitingRoomSlashCommandPolicyDeps,
): Promise<boolean> {
  const trimmed = rawPrompt.trim()
  if (!trimmed.startsWith("/")) {
    return false
  }

  const command = parseSlashCommand(rawPrompt)
  if (!command) {
    deps.flashFooter(`${trimmed} is not wired in the TUI yet`, "error")
    return true
  }

  switch (command.kind) {
    case "waiting":
      deps.flashFooter("already in waiting room", "info")
      clearHandledCommand(deps)
      return true
    case "stop":
      deps.flashFooter("no active prompt", "info")
      clearHandledCommand(deps)
      return true
    case "attachment":
      deps.flashFooter("attachments require an open session", "error")
      clearHandledCommand(deps)
      return true
    case "agent":
    case "workflow":
      deps.flashFooter("start or join a session first", "error")
      clearHandledCommand(deps)
      return true
    default:
      return false
  }
}

function clearHandledCommand(deps: WaitingRoomSlashCommandPolicyDeps) {
  deps.clearPromptText()
  deps.clearCommandCenter()
}
