import type { CommandCenterItem } from "./command-center.js"
import { parseSlashCommand } from "./commands.js"

export function commandCenterCompletionText(item: CommandCenterItem): string {
  if (item.kind === "provider") {
    return `/provider ${item.value}`
  }
  if (item.kind === "model") {
    return `/model ${item.value}`
  }
  if (item.kind === "variant") {
    return `/variant ${item.value}`
  }
  return item.value
}

export function commandCenterExecutionCommand(item: CommandCenterItem): string | null {
  if (item.kind === "command") {
    return item.value
  }
  if (item.kind === "group") {
    return item.value.endsWith(" ") ? null : item.value
  }
  return commandCenterCompletionText(item)
}

export function shouldBypassCommandCenterSubmitSelection(prompt: string): boolean {
  const command = parseSlashCommand(prompt)
  if (!command || command.kind !== "session" || command.action === null) {
    return false
  }
  const action = command.action.toLowerCase()
  return action !== "new" &&
    action !== "create" &&
    action !== "attach" &&
    action !== "list" &&
    action !== "ls" &&
    action !== "delete"
}
