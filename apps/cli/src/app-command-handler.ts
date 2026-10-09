import { open, readFile, rm, stat } from "node:fs/promises"
import { basename } from "node:path"
import { grantAppFileRequest, saveAppFileExportRequest } from "@chariox/kernel-client/ipc-requests"
import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import { tokenizeShellLine } from "@chariox/kernel-client/shell-core"
import { cliOnlyAppVerbs } from "./app-command-catalog.js"
import { acceptAppHostOffer, type AppHostTerminal } from "./app-host-action.js"
import type { AppDevLoop } from "./app-dev-loop.js"
import { AppFileInstaller, FollowLostContact, formatInstallOperation } from "./app-install-file.js"
import { AppPublisherEnrollment, formatPublisherReview } from "./app-publisher-file.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { AppInstallOperationSummary } from "@chariox/kernel-client/kernel-types"

export type AppCommandHandlerDeps = {
  userAppViews?: { handle(args: string[], sessionId?: string): Promise<boolean> }
  appHostTerminal?: AppHostTerminal
  currentAppHostOperationIds?: () => string[]
  lastViewedAppHostOperationId?: () => string | undefined
  appFileInstaller?: AppFileInstaller
  appDevLoop?: AppDevLoop
  appPublisherEnrollment?: AppPublisherEnrollment
  currentAppSessionId?: () => string | undefined
  sendAppRequest?: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

/** `/app` arguments as the TUI sends them. Quoted words are decoded as the
 * shell decodes them (so a copied `app list --after "todo"` works), without
 * variable expansion, as in the web palette. An inbox test's payload is the
 * rest of the line after its five shell words (which may quote IDs with
 * spaces): JSON exactly as typed, so a double-quoted object keeps its quotes
 * and spaces, or a single-quoted word decoded as the shell decoded it. */
export function appSlashArgs(raw: string): string[] {
  const line = raw.trim().replace(/^\/app(?:\s+|$)/, "")
  const [words, payload] = /^inbox\s+test\s/.test(line) ? splitShellWords(line, 5) : [line, ""]
  if (!payload) return tokenizeShellLine(line)
  const quoted = payload.startsWith("'") ? tokenizeShellLine(payload) : null
  return [...tokenizeShellLine(words), quoted?.length === 1 ? quoted[0]! : payload]
}

/** `line` split after its first `count` shell words, with the quoting and
 * escapes of `tokenizeShellLine`; the rest is empty when there is none. */
function splitShellWords(line: string, count: number): [string, string] {
  let quote: string | null = null
  let escaping = false
  let inWord = false
  let words = 0
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index]!
    if (escaping) {
      escaping = false
    } else if (char === "\\" && quote !== "'") {
      escaping = true
    } else if (quote) {
      if (char === quote) quote = null
    } else if (char === "'" || char === '"') {
      quote = char
    } else if (/\s/.test(char)) {
      if (inWord && ++words === count) return [line.slice(0, index), line.slice(index).trimStart()]
      inWord = false
      continue
    }
    inWord = true
  }
  return [line, ""]
}

export async function handleAppSlashCommand(
  deps: AppCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "app" }>,
): Promise<void> {
  if (cliOnlyAppVerbs.has(command.args[0] ?? "")) {
    deps.flashFooter(`app ${command.args[0]} runs in a shell: use chariox app ${command.args[0]}`, "error")
    return
  }
  if (!deps.sendAppRequest) {
    deps.flashFooter("Apps are unavailable in this kernel", "error")
    return
  }
  const args = appSlashArgs(command.raw)
  if (deps.userAppViews && await deps.userAppViews.handle(args, deps.currentAppSessionId?.())) return
  if (args[0] === "view" || args[0] === "views") {
    throw new Error("Enable CHARIOX_USER_APP_VIEWS_PROTOTYPE=1 to use the App text prototype")
  }
  if (command.args[0] === "host") {
    const [, action, explicitOperation, ...extra] = tokenizeShellLine(command.raw.replace(/^\/app(?:\s|$)/, ""))
    if (action !== "accept" || extra.length) throw new Error("usage: /app host accept [OPERATION]")
    const session = deps.currentAppSessionId?.()
    if (!session) throw new Error("Attach to the session showing the App host request")
    const candidates = explicitOperation ? [explicitOperation] : deps.currentAppHostOperationIds?.() ?? []
    if (candidates.length !== 1) throw new Error(candidates.length ? "Several App host requests are pending; use /app host accept OPERATION" : "No App host request is pending in this session")
    if (!explicitOperation && candidates[0] !== deps.lastViewedAppHostOperationId?.()) throw new Error("Open kernel approvals (F8), review the App host offer, close the panel (Esc), then use /app host accept")
    await acceptAppHostOffer(session, candidates[0]!, deps.sendAppRequest, deps.appendNotice, deps.appHostTerminal)
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
  // `/app file revoke` reads no local file: the shared App command sends it.
  if (command.args[0] === "file" && command.args[1] !== "revoke") {
    // Tokenization preserves quoted local paths; only each file's name and
    // bytes are sent, never its path.
    const [, action, operation, ...paths] = tokenizeShellLine(command.raw.replace(/^\/app(?:\s|$)/, ""))
    const session = deps.currentAppSessionId?.()
    if (action === "save" && operation && paths.length === 1 && paths[0]) {
      if (!session) throw new Error("Attach to the session showing the file offer")
      // Create the destination first (never replacing a file), so a bad path
      // fails before the offer is taken.
      const output = await open(paths[0], "wx")
      try {
        const response = await deps.sendAppRequest(saveAppFileExportRequest(session, operation))
        const offered = response.AppFileExport as { name?: string; contents_base64?: string } | undefined
        if (typeof offered?.contents_base64 !== "string") {
          const code = (response.AppRequestFailed as { code?: string } | undefined)?.code
          throw new Error(code === "conflict" ? "That file offer was declined or has ended"
            : code === "not_found" ? "No such file offer for you" : "Saving the file failed")
        }
        await output.writeFile(Buffer.from(offered.contents_base64, "base64"))
        await output.close()
        // The prompt closes now; the offer can still be taken until it ends.
        deps.appendNotice(`Saved ${offered.name ?? "the file"} to ${paths[0]}. To save it again before the offer ends: /app file save ${operation} "PATH"`)
      } catch (error) {
        await output.close().catch(() => {})
        await rm(paths[0], { force: true })
        throw error
      }
      return
    }
    if (action !== "grant" || !operation || paths.length === 0 || paths.length > 8) {
      throw new Error('usage: /app file grant OPERATION "FILE" ["FILE"...] | /app file save OPERATION "PATH" | /app file revoke INSTALLATION [OPERATION]')
    }
    if (!session) throw new Error("Attach to the session showing the file request")
    const files = []
    let total = 0
    for (const path of paths) {
      const size = (await stat(path)).size
      if (size > 512 * 1024) throw new Error(`${basename(path)} is larger than 512 KiB`)
      total += size
      if (total > 512 * 1024) throw new Error("The chosen files are larger than 512 KiB together")
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
      reportOutcome(installer, await installer.install(args[0], session), deps)
    } else if (action === "update") {
      if (args.length !== 2 || !args[0] || !args[1]) throw new Error('usage: /app update INSTALLATION "FILE.cxapp"')
      const session = deps.currentAppSessionId?.()
      if (!session) throw new Error("Attach to a session before updating an App")
      deps.appendNotice("Reading and uploading App update. Use /app cancel to stop the transfer.")
      reportOutcome(installer, await installer.update(args[0], args[1], session), deps)
    } else {
      if (args.length > 1) throw new Error(`usage: /app ${action} [request-id]`)
      const value = action === "cancel" ? await installer.cancel(args[0]) : await installer.status(args[0])
      deps.appendNotice(value ? formatInstallOperation(value) : "App upload cancelled.")
    }
    return
  }
  const result = await executeAppCommand(appSlashArgs(command.raw), { send: deps.sendAppRequest }, {
    sessionId: deps.currentAppSessionId?.(), appCommandPrefix: "/app",
  })
  if (!result.ok) {
    deps.flashFooter(result.message ?? "App command failed", "error")
    return
  }
  if (result.message) deps.appendNotice(result.message)
}

/** Shows a started install or update, then follows it in the background and
 * shows each phase it reaches through its outcome, as the web terminal does.
 * The prompt stays free for the approval it may wait for. */
function reportOutcome(installer: AppFileInstaller, value: AppInstallOperationSummary, deps: AppCommandHandlerDeps): void {
  deps.appendNotice(formatInstallOperation(value))
  // A re-run of the same command joins the running follow, which already reports.
  if (installer.isFollowing(value.request_id)) return
  void installer.follow(value, next => deps.appendNotice(formatInstallOperation(next))).catch((error: unknown) =>
    deps.appendNotice(error instanceof FollowLostContact
      ? `${error.message} Use /app operation ${value.request_id} to check it.`
      : `Stopped following App operation ${value.request_id}: ${error instanceof Error ? error.message : String(error)}`))
}
