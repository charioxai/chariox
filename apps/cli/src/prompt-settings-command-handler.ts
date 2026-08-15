import { executePromptSettingsCommand } from "@chariox/kernel-client"
import type { ParsedSlashCommand } from "./commands.js"

type PromptSettingsCommandHandlerDeps = {
  sendWorkflowEventPublicationRequest?: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handlePromptSettingsSlashCommand(
  deps: PromptSettingsCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "settings" }>,
): Promise<void> {
  if (!deps.sendWorkflowEventPublicationRequest) {
    deps.flashFooter("prompt settings are unavailable in this kernel", "error")
    return
  }
  const result = await executePromptSettingsCommand(command.args.slice(1), { send: deps.sendWorkflowEventPublicationRequest })
  if (!result.ok) {
    deps.flashFooter(result.message ?? "prompt settings command failed", "error")
    return
  }
  if (result.message) deps.appendNotice(result.message)
}
