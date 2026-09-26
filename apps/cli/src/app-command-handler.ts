import { readFile, stat } from "node:fs/promises"
import { basename } from "node:path"
import { grantAppFileRequest } from "@chariox/kernel-client/ipc-requests"
import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import { tokenizeShellLine } from "@chariox/kernel-client/shell-core"
import type { AppDevLoop } from "./app-dev-loop.js"
import { AppFileInstaller, formatInstallOperation } from "./app-install-file.js"
import { AppPublisherEnrollment, formatPublisherReview } from "./app-publisher-file.js"
import type { ParsedSlashCommand } from "./commands.js"

export type AppCommandHandlerDeps = {
  appFileInstaller?: AppFileInstaller
  appDevLoop?: AppDevLoop
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
  if (command.args[0] === "dev") {
    const loop = deps.appDevLoop
    if (!loop) throw new Error("The App dev loop is unavailable in this terminal")
    const [, ...args] = tokenizeShellLine(command.raw.replace(/^\/app(?:\s|$)/, ""))
    if (args.length === 1 && args[0] === "stop") {
      deps.appendNotice(await loop.stop() ? "App dev loop stopped." : "No App dev loop is running in this terminal.")
      return
    }
    const usage = 'usage: /app dev "DIRECTORY" [--key PRIVATE] | /app dev stop'
    if (!args[0] || args[0].startsWith("--") || !(args.length === 1 || (args.length === 3 && args[1] === "--key" && args[2]))) throw new Error(usage)
    await loop.start(args[0], args[2] ? { key: args[2] } : {})
    return
  }
  if (command.args[0] === "file") {
    // Tokenization preserves quoted local paths; only each file's name and
    // bytes are sent, never its path.
    const [, action, operation, ...paths] = tokenizeShellLine(command.raw.replace(/^\/app(?:\s|$)/, ""))
    if (action !== "grant" || !operation || paths.length === 0 || paths.length > 8) {
      throw new Error('usage: /app file grant OPERATION "FILE" ["FILE"...]')
    }
    const session = deps.currentAppSessionId?.()
    if (!session) throw new Error("Attach to the session showing the file request")
    const files = []
    for (const path of paths) {
      if ((await stat(path)).size > 512 * 1024) throw new Error(`${basename(path)} is larger than 512 KiB`)
      files.push({ name: basename(path), contentsBase64: (await readFile(path)).toString("base64") })
    }
    const response = await deps.sendAppRequest(grantAppFileRequest(session, operation, files))
    const granted = response.AppFileGranted as { files?: number } | undefined
    if (!granted) {
      const code = (response.AppRequestFailed as { code?: string } | undefined)?.code
      throw new Error(code === "not_found" ? "No such file request for you" : code === "conflict"
        ? "That file request was already answered or expired" : code === "invalid_request"
          ? "The App does not accept these files" : "Sharing the files failed")
    }
    deps.appendNotice(`Shared ${granted.files} file${granted.files === 1 ? "" : "s"} with the App.`)
    return
  }
  if (["install", "update", "operation", "cancel"].includes(command.args[0] ?? "")) {
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
    } else if (action === "update") {
      if (args.length !== 2 || !args[0] || !args[1]) throw new Error('usage: /app update INSTALLATION "FILE.cxapp"')
      const session = deps.currentAppSessionId?.()
      if (!session) throw new Error("Attach to a session before updating an App")
      deps.appendNotice("Reading and uploading App update. Use /app cancel to stop the transfer.")
      deps.appendNotice(formatInstallOperation(await installer.update(args[0], args[1], session)))
    } else {
      if (args.length > 1) throw new Error(`usage: /app ${action} [request-id]`)
      const value = action === "cancel" ? await installer.cancel(args[0]) : await installer.status(args[0])
      deps.appendNotice(value ? formatInstallOperation(value) : "App upload cancelled.")
    }
    return
  }
  const result = await executeAppCommand(command.args, { send: deps.sendAppRequest }, { sessionId: deps.currentAppSessionId?.() })
  if (!result.ok) {
    deps.flashFooter(result.message ?? "App command failed", "error")
    return
  }
  if (result.message) deps.appendNotice(result.message)
}
