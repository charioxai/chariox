import { writeSync } from "node:fs"

// MP-08/MP-11: OpenTUI's destroy() calls setRawMode(false) before it writes any
// mode reset. After the launcher is SIGKILLed the user's shell takes the
// terminal back, this process is a background group member, and that call
// fails with EIO, so none of the resets are written. Plain writes stay allowed.
export const TERMINAL_MODE_RESTORE_SEQUENCE = [
  "\u001b[?1000l", "\u001b[?1002l", "\u001b[?1003l", "\u001b[?1006l", // mouse
  "\u001b[?1004l", "\u001b[?2004l", "\u001b[?2031l", "\u001b[?2027l", "\u001b[?2026l",
  "\u001b[>4;0m", // modifyOtherKeys
  "\u001b[?1049l", // alternate screen
  "\u001b[?25h", // cursor
].join("")

export function writeTerminalModeRestore(): void {
  if (process.stdout.isTTY) writeSync(process.stdout.fd, TERMINAL_MODE_RESTORE_SEQUENCE)
}

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
  restoreTerminalModes?: () => void
  writeMessage?: (message: string) => void
}

export type TerminalExitController = {
  restoreAndExit(exitCode: number, message?: string): Promise<never>
}

export function createTerminalExitController(
  options: TerminalExitControllerOptions,
): TerminalExitController {
  return {
    async restoreAndExit(exitCode, message) {
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
        try {
          options.restoreTerminalModes?.()
        } catch {}
      }
      // The alternate screen discards output, so the reason follows restoration.
      if (message !== undefined) {
        try {
          options.writeMessage?.(message)
        } catch {}
      }
      await options.sleep(25)
      options.exitProcess(exitCode)
    },
  }
}
