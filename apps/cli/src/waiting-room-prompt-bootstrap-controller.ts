export type WaitingRoomPromptBootstrapResult = "bootstrapped" | "handled" | "unhandled"

export type WaitingRoomPromptBootstrapControllerDeps = {
  isAttached: () => boolean
  startSessionFromWaitingRoomDefaults?: () => Promise<unknown>
  flashFooter: (message: string, tone: "info" | "error") => void
  formatError?: (error: unknown) => string
  warn?: (message: string, fields: Record<string, unknown>) => void
}

export type WaitingRoomPromptBootstrapController = {
  bootstrap(): Promise<WaitingRoomPromptBootstrapResult>
}

export function createWaitingRoomPromptBootstrapController(
  deps: WaitingRoomPromptBootstrapControllerDeps,
): WaitingRoomPromptBootstrapController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))
  let inFlight: Promise<WaitingRoomPromptBootstrapResult> | null = null

  return {
    async bootstrap() {
      if (deps.isAttached()) {
        return "unhandled"
      }
      if (!deps.startSessionFromWaitingRoomDefaults) {
        return "unhandled"
      }
      if (inFlight) {
        deps.flashFooter("starting session", "info")
        return "handled"
      }

      inFlight = deps.startSessionFromWaitingRoomDefaults()
        .then(() => "bootstrapped" as const)
        .catch((error) => {
          deps.warn?.("waiting-room prompt bootstrap failed", { error: formatError(error) })
          deps.flashFooter(formatError(error), "error")
          return "handled" as const
        })
        .finally(() => {
          inFlight = null
        })
      return await inFlight
    },
  }
}
