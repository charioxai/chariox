export type TerminalResizeControllerDeps = {
  isAttached: () => boolean
  sessionId: () => string
  resizeSession: (sessionId: string) => Promise<unknown> | unknown
  /** The waiting room is drawn to the terminal width at build time. */
  rebuildWaitingRoom: () => void
}

export function createTerminalResizeController(
  deps: TerminalResizeControllerDeps,
) {
  return {
    handleResize() {
      if (!deps.isAttached()) {
        deps.rebuildWaitingRoom()
        return false
      }
      void deps.resizeSession(deps.sessionId())
      return true
    },
  }
}
