type TerminalExitRenderer = {
  isDestroyed?: boolean
  disableKittyKeyboard: () => void
  disableStdoutInterception: () => void
  destroy: () => void
}

type TerminalExitControllerOptions = {
  renderer: TerminalExitRenderer
  sleep: (delayMs: number) => Promise<void>
  exitProcess: (exitCode: number) => never
  onRendererDestroyFailed: (error: unknown) => void
}

export type TerminalExitController = {
  restoreAndExit(exitCode: number): Promise<never>
}

export function createTerminalExitController(
  options: TerminalExitControllerOptions,
): TerminalExitController {
  return {
    async restoreAndExit(exitCode) {
      try {
        options.renderer.disableKittyKeyboard()
      } catch {}
      try {
        options.renderer.disableStdoutInterception()
      } catch {}
      try {
        if (!options.renderer.isDestroyed) {
          options.renderer.destroy()
        }
      } catch (error) {
        options.onRendererDestroyFailed(error)
      }
      await options.sleep(25)
      options.exitProcess(exitCode)
    },
  }
}
