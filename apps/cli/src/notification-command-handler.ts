import { executeNotificationCommand } from "@chariox/kernel-client"
import type { ParsedSlashCommand } from "./commands.js"

export type NotificationCommandHandlerDeps = {
  sendWorkflowEventPublicationRequest?: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleNotificationSlashCommand(
  deps: NotificationCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "notifications" }>,
): Promise<void> {
  if (!deps.sendWorkflowEventPublicationRequest) {
    deps.flashFooter("notifications are unavailable in this kernel", "error")
    return
  }
  const result = await executeNotificationCommand(
    command.args,
    { send: deps.sendWorkflowEventPublicationRequest },
  )
  if (!result.ok) {
    deps.flashFooter(result.message ?? "notification command failed", "error")
    return
  }
  if (result.message) deps.appendNotice(result.message)
}
