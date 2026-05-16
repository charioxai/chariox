import type { RuntimeSession } from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"

type FooterTone = "info" | "error"

export type KernelCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  flashFooter: (message: string, tone: FooterTone) => void
  deleteKernel?: () => Promise<{ kernelId: string; deletedSessions: RuntimeSession[] }>
  transitionToNoSession: (message: string) => void
}

export async function handleKernelSlashCommand(
  deps: KernelCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "kernel" }>,
): Promise<void> {
  const [subcommand, ...args] = command.args
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
  deps.flashFooter("usage: /kernel delete", "error")
}
