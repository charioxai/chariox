import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import { tokenizeShellLine } from "@chariox/kernel-client/shell-core"
import { AppFileInstaller, formatInstallOperation } from "./app-install-file.js"
import { AppPublisherEnrollment, formatPublisherReview } from "./app-publisher-file.js"
import type { ParsedSlashCommand } from "./commands.js"

export type AppCommandHandlerDeps = {
  appFileInstaller?: AppFileInstaller
  appPublisherEnrollment?: AppPublisherEnrollment
  currentAppSessionId?: () => string | undefined
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
  if (command.args[0] === "publisher") {
    const enrollment = deps.appPublisherEnrollment
    if (!enrollment) throw new Error("Publisher enrollment is unavailable in this terminal")
    const [, action, ...args] = tokenizeShellLine(command.raw.replace(/^\/app(?:\s|$)/, ""))
    if (action === "enroll") {
      if (!args[0] || (args.length !== 1 && !(args.length === 3 && args[1] === "--revision"))) throw new Error('usage: /app publisher enroll "publisher.json" [--revision N]')
      const session = deps.currentAppSessionId?.()
      if (!session) throw new Error("Attach to a session before enrolling a publisher")
      deps.appendNotice(formatPublisherReview(await enrollment.enroll(args[0], session, args[2])))
    } else if (action === "status" || action === "cancel") {
      if (args.length > 1) throw new Error(`usage: /app publisher ${action} [review-id]`)
      deps.appendNotice(formatPublisherReview(await enrollment[action](args[0])))
    } else throw new Error("usage: /app publisher enroll|status|cancel")
    return
  }
  if (["install", "operation", "cancel"].includes(command.args[0] ?? "")) {
    const installer = deps.appFileInstaller
    if (!installer) throw new Error("File installation is unavailable in this terminal")
    // Tokenization preserves quoted local paths; it never expands variables or executes a shell.
    const [action, ...args] = tokenizeShellLine(command.raw.replace(/^\/app(?:\s|$)/, ""))
    if (action === "install") {
      if (args.length !== 1 || !args[0]) throw new Error('usage: /app install "FILE.cxapp"')
      const session = deps.currentAppSessionId?.()
      if (!session) throw new Error("Attach to a session before installing an App")
      deps.appendNotice("Reading and uploading App. Use /app cancel to stop the transfer.")
      const value = await installer.install(args[0], session)
      deps.appendNotice(formatInstallOperation(value))
    } else {
      if (args.length > 1) throw new Error(`usage: /app ${action} [request-id]`)
      const value = action === "cancel" ? await installer.cancel(args[0]) : await installer.status(args[0])
      deps.appendNotice(value ? formatInstallOperation(value) : "App upload cancelled.")
    }
    return
  }
  const result = await executeAppCommand(command.args, { send: deps.sendAppRequest })
  if (!result.ok) {
    deps.flashFooter(result.message ?? "App command failed", "error")
    return
  }
  if (result.message) deps.appendNotice(result.message)
}
