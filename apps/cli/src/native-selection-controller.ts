import { NATIVE_SELECTION_HINT } from "./clipboard.js"

type SelectionKey = { name: string; eventType?: string; ctrl?: boolean }

/** MP-08 / MP-10: the raw stdin controller owns the toggle; the renderer
 * suppresses app input while the terminal owns drag selection and Cmd-C. */
export function createNativeSelectionController(deps: {
  renderer: { useMouse: boolean; clearSelection(): void }
  setHint(hint: string | null): void
}) {
  let active = false
  let previousMouse = deps.renderer.useMouse
  const leave = () => {
    if (!active) return
    active = false
    deps.renderer.useMouse = previousMouse
    deps.setHint(null)
  }
  return {
    isActive: () => active,
    // Installed before the renderer's key dispatcher; raw stdin runs afterwards.
    ownsRendererKey: (event: SelectionKey) => event.name === "f7" || active,
    handleKey(event: SelectionKey) {
      if (event.name === "f7" || (active && event.name === "escape")) {
        if (event.eventType === "release") return true
        if (active) leave()
        else {
          previousMouse = deps.renderer.useMouse
          active = true
          deps.renderer.clearSelection()
          deps.renderer.useMouse = false
          deps.setHint(NATIVE_SELECTION_HINT)
        }
        return true
      }
      // Keep typing/paste/approval shortcuts from changing the selection view.
      // Ctrl+E remains available as the ordinary explicit exit shortcut.
      return active && !(event.ctrl && event.name === "e")
    },
    dispose: leave,
  }
}
