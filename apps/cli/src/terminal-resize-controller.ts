export type TerminalResizeControllerDeps = {
  isAttached: () => boolean
  sessionId: () => string
  resizeSession: (sessionId: string) => Promise<unknown> | unknown
}

export function createTerminalResizeController(
  deps: TerminalResizeControllerDeps,
) {
  return {
    handleResize() {
      if (!deps.isAttached()) {
        return false
      }
      void deps.resizeSession(deps.sessionId())
      return true
    },
  }
}
