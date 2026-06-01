import type { RuntimeSession } from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import { defaultLogDir } from "./logging.js"
import { matchingLogRecords, writeLogBundle } from "./logs.js"
import path from "node:path"
import {
  formatKernelHealth,
  kernelHealthIssueCount,
  type DaemonHealthProjection,
} from "@arroba/kernel-client"

export {
  formatKernelHealth,
  kernelHealthIssueCount,
}

type FooterTone = "info" | "error"

export type KernelCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  flashFooter: (message: string, tone: FooterTone) => void
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  getDaemonHealth?: () => Promise<DaemonHealthProjection>
  writeDebugLogBundle?: (input: {
    sessionId: string
    bundleDir: string
    limit: number
  }) => { bundleDir: string; recordCount: number }
  appendNotice: (message: string) => void
  transitionToNoSession: (message: string) => void
}

export async function handleKernelSlashCommand(
  deps: KernelCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "kernel" }>,
): Promise<void> {
  const [subcommand, ...args] = command.args
  if (subcommand === "health" || subcommand === "status") {
    if (!deps.getDaemonHealth) {
      deps.flashFooter("kernel health is unavailable in this build", "error")
      return
    }
    if (args.length > 0) {
      deps.flashFooter("usage: /kernel health", "error")
      return
    }
    const health = await deps.getDaemonHealth()
    const issueCount = kernelHealthIssueCount(health)
    deps.appendNotice(formatKernelHealth(health))
    deps.flashFooter(
      issueCount === 0
        ? "kernel health: ok"
        : `kernel health: ${issueCount} issue${issueCount === 1 ? "" : "s"}`,
      issueCount === 0 ? "info" : "error",
    )
    return
  }
  if (subcommand === "debug-bundle") {
    if (!deps.isAttached()) {
      deps.flashFooter("attach to a session before writing a debug bundle", "error")
      return
    }
    if (args.length > 1) {
      deps.flashFooter("usage: /kernel debug-bundle [directory]", "error")
      return
    }
    const session = deps.sessionState()
    const bundleDir = args[0] ?? defaultDebugBundleDir(session.id)
    const bundle = deps.writeDebugLogBundle
      ? deps.writeDebugLogBundle({ sessionId: session.id, bundleDir, limit: 1000 })
      : writeAttachedSessionDebugBundle(session.id, bundleDir, 1000)
    deps.appendNotice([
      `debug bundle: ${bundle.bundleDir}`,
      `session: ${session.id}`,
      `records: ${bundle.recordCount}`,
      "contents: manifest.json, logs.ndjson",
    ].join("\n"))
    deps.flashFooter(`debug bundle written: ${bundle.recordCount} records`, "info")
    return
  }
  if (subcommand === "delete") {
    if (!deps.deleteKernel) {
      deps.flashFooter("kernel delete is unavailable in this build", "error")
      return
    }
    if (args.length > 0) {
      deps.flashFooter("usage: /kernel delete", "error")
      return
    }
    const deleted = await deps.deleteKernel()
    if (deps.isAttached() && deleted.deletedSessions.some((session) => session.id === deps.sessionState().id)) {
      deps.transitionToNoSession(`Kernel ${deleted.kernelId} was deleted.`)
      return
    }
    deps.flashFooter(`deleted kernel ${deleted.kernelId} (${deleted.deletedSessions.length} session${deleted.deletedSessions.length === 1 ? "" : "s"})`, "info")
    return
  }
  deps.flashFooter("usage: /kernel health | /kernel debug-bundle [directory] | /kernel delete", "error")
}

function writeAttachedSessionDebugBundle(sessionId: string, bundleDir: string, limit: number) {
  const logDir = defaultLogDir()
  const options = { sessionId, limit }
  const records = matchingLogRecords(logDir, options)
  return writeLogBundle(logDir, bundleDir, options, records)
}

function defaultDebugBundleDir(sessionId: string) {
  const safeSession = sessionId.replace(/[^a-zA-Z0-9._-]/g, "_") || "session"
  return path.join(process.cwd(), ".arroba", "debug-bundles", `${safeSession}-${Date.now()}`)
}
