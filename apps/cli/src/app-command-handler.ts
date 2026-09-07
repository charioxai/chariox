import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import type { ParsedSlashCommand } from "./commands.js"

export type AppCommandHandlerDeps = {
  sendAppRequest?: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleAppSlashCommand(
  deps: AppCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "app" }>,
): Promise<void> {
  if (!deps.sendAppRequest) {
    deps.flashFooter("Apps are unavailable in this kernel", "error")
    return
  }
  const result = await executeAppCommand(command.args, { send: deps.sendAppRequest })
  if (!result.ok) {
    deps.flashFooter(result.message ?? "App command failed", "error")
    return
  }
  if (result.message) deps.appendNotice(result.message)
}
