import type { RuntimeSession } from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import {
  formatKernelHealth,
  formatKernelRemoteRuntimeHealth,
  kernelHealthIssueCount,
  kernelRemoteRuntimeIssueCount,
  kernelRemoteRuntimeReadiness,
  type DaemonHealthProjection,
} from "@chariox/kernel-client"

export {
  formatKernelHealth,
  formatKernelRemoteRuntimeHealth,
  kernelHealthIssueCount,
  kernelRemoteRuntimeIssueCount,
  kernelRemoteRuntimeReadiness,
}

type FooterTone = "info" | "error"

export type KernelCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  flashFooter: (message: string, tone: FooterTone) => void
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  getDaemonHealth?: () => Promise<DaemonHealthProjection>
  exportDebugBundle?: (sessionId: string, label: string | null) => Promise<{
    bundleDir: string
    recordCount: number
    limit: number
  }>
  appendNotice: (message: string) => void
  transitionToNoSession: (message: string) => void
}

export async function handleKernelSlashCommand(
  deps: KernelCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "kernel" }>,
): Promise<void> {
  const [subcommand, ...args] = command.args
  if (subcommand === "health" || subcommand === "status" || subcommand === "remote-runtime" || subcommand === "runtime") {
    if (!deps.getDaemonHealth) {
      deps.flashFooter("kernel health is unavailable in this build", "error")
      return
    }
    if (args.length > 0) {
      deps.flashFooter("usage: /kernel health | /kernel remote-runtime", "error")
      return
    }
    const health = await deps.getDaemonHealth()
    const remoteRuntime = subcommand === "remote-runtime" || subcommand === "runtime"
    const remoteReadiness = remoteRuntime ? kernelRemoteRuntimeReadiness(health) : null
    const issueCount = remoteReadiness?.attentionCount ?? kernelHealthIssueCount(health)
    deps.appendNotice(remoteRuntime ? formatKernelRemoteRuntimeHealth(health) : formatKernelHealth(health))
    const label = remoteRuntime ? "remote runtime" : "kernel health"
    const footerMessage = remoteReadiness
      ? remoteReadiness.state === "ok"
        ? `${label}: ok`
        : remoteReadiness.state === "degraded"
          ? `${label}: degraded (${remoteReadiness.attentionCount} attention)`
          : `${label}: ${remoteReadiness.issueCount} issue${remoteReadiness.issueCount === 1 ? "" : "s"}`
      : issueCount === 0
        ? `${label}: ok`
        : `${label}: ${issueCount} issue${issueCount === 1 ? "" : "s"}`
    deps.flashFooter(
      footerMessage,
      issueCount === 0 ? "info" : "error",
    )
    return
  }
  if (subcommand === "debug-bundle") {
    if (!deps.exportDebugBundle) {
      deps.flashFooter("kernel debug bundle export is unavailable in this build", "error")
      return
    }
    if (!deps.isAttached()) {
      deps.flashFooter("attach to a session before exporting a debug bundle", "error")
      return
    }
    if (args.length > 1) {
      deps.flashFooter("usage: /kernel debug-bundle [label]", "error")
      return
    }
    const session = deps.sessionState()
    const bundle = await deps.exportDebugBundle(session.id, args[0] ?? null)
    deps.appendNotice([
      `debug bundle: ${bundle.bundleDir}`,
      "location: kernel machine",
      `session: ${session.id}`,
      `records: ${bundle.recordCount}/${bundle.limit}`,
      "contents: manifest.json, logs.ndjson",
    ].join("\n"))
    deps.flashFooter(`debug bundle exported: ${bundle.recordCount} records`, "info")
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
  deps.flashFooter("usage: /kernel health | /kernel remote-runtime | /kernel debug-bundle [label] | /kernel delete", "error")
}
