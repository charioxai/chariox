import type { CommandCenterItem } from "./command-center-types.js"
import { parseSlashCommand } from "./commands.js"
import { formatPromptAgentAliasAddress } from "@arroba/kernel-client/prompt-submission"

export function commandCenterCompletionText(item: CommandCenterItem): string {
  if (item.kind === "agent") {
    return `${formatPromptAgentAliasAddress(item.value)} `
  }
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
  if (item.kind === "agent") {
    return null
  }
  if (item.kind === "command") {
    return item.value
  }
  if (item.kind === "group") {
    return item.value.endsWith(" ") ? null : item.value
  }
  return commandCenterCompletionText(item)
}

export function shouldSubmitExactCommandCenterMatch(item: CommandCenterItem, currentPrompt: string) {
  if (item.kind !== "command") {
    return false
  }
  if (!item.value.endsWith(" ")) {
    return currentPrompt.trim() === item.value
  }
  return currentPrompt.startsWith(item.value) || currentPrompt === item.value.trim()
}

export function nextCommandCenterIndex(
  currentIndex: number,
  items: readonly CommandCenterItem[],
  input: string,
  previousInput?: string,
) {
  if (items.length === 0) {
    return 0
  }

  if (previousInput !== input) {
    const normalized = input.trim()
    if (normalized === "/" || input.startsWith("@")) {
      return 0
    }
    const exactGroupIndex = items.findIndex((item) => (
      item.kind === "group"
      && (normalized === item.value.trim() || input === item.value)
    ))
    if (exactGroupIndex >= 0) {
      return exactGroupIndex
    }
  }

  return Math.max(0, Math.min(currentIndex, items.length - 1))
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
