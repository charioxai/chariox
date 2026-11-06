import type { KeyHandler } from "@opentui/core"
import type { CliStdinInputEvent, CliStdinKeyEvent } from "./cli-stdin-key-controller.js"

/** MP-08 / MP-10: one decoder owns framing, mouse resets and suspension.
 * Subscribe before native/approval handlers can stop propagation, then replay
 * after renderer dispatch so selection snapshots and ownership are ready. */
export function bindRendererShortcutInput(deps: {
  keyInput: KeyHandler
  enabled(): boolean
  handleEvent(event: CliStdinInputEvent): boolean
  discardInput(): void
}) {
  let disposed = false
  const defer = (event: CliStdinInputEvent) => {
    const enabled = deps.enabled()
    queueMicrotask(() => {
      if (disposed) return
      if (enabled && deps.enabled()) deps.handleEvent(event)
      else deps.discardInput()
    })
  }
  const key = (event: CliStdinKeyEvent) => defer({ type: "key", key: event })
  const paste = () => defer({ type: "paste" })
  deps.keyInput.prependListener("keypress", key)
  deps.keyInput.prependListener("keyrelease", key)
  deps.keyInput.prependListener("paste", paste)
  return () => {
    disposed = true
    deps.keyInput.off("keypress", key)
    deps.keyInput.off("keyrelease", key)
    deps.keyInput.off("paste", paste)
  }
}
