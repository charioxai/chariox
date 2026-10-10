import { NATIVE_SELECTION_HINT } from "./clipboard.js"

type SelectionKey = { name: string; eventType?: string; ctrl?: boolean; sequence?: string }

/** MP-08 / MP-10: transition in renderer event order, then replay ownership
 * to the raw listener, which runs after the renderer drains the whole chunk. */
export function createNativeSelectionController(deps: {
  renderer: { useMouse: boolean; clearSelection(): void }
  setHint(hint: string | null): void
}) {
  let active = false
  let previousMouse = deps.renderer.useMouse
  const rendererKeys: Array<{ key: SelectionKey; owned: boolean }> = []
  const rendererPastes: boolean[] = []
  const leave = () => {
    if (!active) return
    active = false
    deps.renderer.useMouse = previousMouse
    deps.setHint(null)
  }
  const handleKey = (event: SelectionKey): boolean => {
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
  }
  return {
    isActive: () => active,
    handleRendererKey(event: SelectionKey) {
      const owned = handleKey(event)
      rendererKeys.push({ key: event, owned })
      return owned
    },
    handleRendererPaste() {
      rendererPastes.push(active)
      return active
    },
    // Preserve each event's decision even if another F7 in the same chunk
    // has already changed the renderer's final state. Do not toggle twice.
    handleKey(event: SelectionKey) {
      const next = rendererKeys[0]
      // The renderer may consume a terminal response in a sequence handler
      // without emitting a key. Such raw events must not steal the next key.
      if (next && next.key.name === event.name && next.key.sequence === event.sequence) {
        return rendererKeys.shift()!.owned
      }
      return event.name === "f7" || (active && !(event.ctrl && event.name === "e"))
    },
    handlePaste: () => rendererPastes.shift() ?? active,
    discardInput() { rendererKeys.length = 0; rendererPastes.length = 0 },
    dispose() { leave(); rendererKeys.length = 0; rendererPastes.length = 0 },
  }
}
