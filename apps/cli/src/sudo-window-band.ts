import { BoxRenderable, MouseButton, TextAttributes, TextRenderable, type CliRenderer } from "@opentui/core"
import type { KernelSudoTurn } from "@chariox/kernel-client/kernel-types"
import type { KernelApprovalKey } from "./kernel-approval-controller.js"
import { sudoWindowLine } from "./sudo-window-line.js"
import { theme } from "./theme.js"

/** MP-08/MP-10/MP-11 G06: the mouse is off by default, so F9 moves the
 * keyboard into the band: Left/Right pick Extend or Revoke, Up/Down pick the
 * window, Enter runs it, Esc gives the keyboard back. */
export const SUDO_BAND_KEY = "f9"

type BandFocus = { window: number; control: number }

export function createSudoWindowBand(renderer: CliRenderer, actions: {
  extend(window: KernelSudoTurn): void
  revoke(window: KernelSudoTurn): void
  /** The band took or gave back the keyboard. */
  focusChanged?(focused: boolean): void
}) {
  const controls = [["[Extend]", actions.extend], ["[Revoke]", actions.revoke]] as const
  let band: BoxRenderable | undefined
  let lastFrame = ""
  let windows: readonly KernelSudoTurn[] = []
  let now = 0
  let agentLabel: (agentId: string) => string = (agentId) => agentId
  let focus: BandFocus | null = null
  const setFocus = (next: BandFocus | null) => {
    const changed = (focus === null) !== (next === null)
    focus = next
    draw()
    if (changed) actions.focusChanged?.(next !== null)
  }
  function draw() {
    if (!band) return
    const lines = windows.map((window) => sudoWindowLine(window, now, agentLabel(window.agent_id)))
    // MP-08/MP-10/MP-11 P3: handlers capture the window, so its revision
    // must invalidate the frame even when the rounded deadline is unchanged.
    const frame = JSON.stringify([lines, windows.map((window) => window.revision), focus, theme.warning, theme.primary, theme.textMuted, theme.backgroundElement])
    // MP-08/MP-10/MP-11: a layout mount can hide the box after its ref;
    // cached content still has to restore the current visibility.
    const visibilityChanged = band.visible !== (windows.length > 0)
    band.visible = windows.length > 0
    if (frame === lastFrame) {
      if (visibilityChanged) band.requestRender()
      return
    }
    lastFrame = frame
    for (const child of [...band.getChildren()]) {
      band.remove(String(child.id))
      child.destroyRecursively()
    }
    band.backgroundColor = theme.backgroundElement
    band.paddingLeft = 1
    band.paddingRight = 1
    windows.forEach((window, index) => {
      const row = new BoxRenderable(renderer, { flexDirection: "row", gap: 2, flexShrink: 0 })
      row.add(new TextRenderable(renderer, {
        content: lines[index]!, flexShrink: 1, wrapMode: "word",
        fg: window.warning_sent ? theme.error : theme.warning, attributes: TextAttributes.BOLD,
      }))
      controls.forEach(([label, action], control) => {
        const selected = focus?.window === index && focus.control === control
        const text = new TextRenderable(renderer, {
          content: selected ? `> ${label}` : label, fg: theme.primary, flexShrink: 0,
          ...(selected ? { attributes: TextAttributes.BOLD } : {}),
        })
        text.onMouseUp = (event) => {
          event.stopPropagation()
          if (event.button === MouseButton.LEFT) action(window)
        }
        row.add(text)
      })
      row.add(new TextRenderable(renderer, {
        content: focus ? "←/→ choose · ↑/↓ window · Enter run · Esc close" : "F9 keys",
        fg: theme.textMuted, flexShrink: 0,
      }))
      band!.add(row)
    })
    band.requestRender()
  }
  return {
    assign(value: BoxRenderable) { band = value; lastFrame = "" },
    render(next: readonly KernelSudoTurn[], at: number, label: (agentId: string) => string) {
      windows = next
      now = at
      agentLabel = label
      // The window under the keyboard ended: nothing is left to act on.
      if (focus && focus.window >= windows.length) {
        if (windows.length === 0) setFocus(null)
        else setFocus({ ...focus, window: windows.length - 1 })
        return
      }
      draw()
    },
    ownsInput: () => focus !== null,
    handleKey(event: KernelApprovalKey): boolean {
      if (!focus) {
        if (event.name !== SUDO_BAND_KEY || event.ctrl || event.meta || event.alt || event.shift || !windows.length) return false
        event.preventDefault?.()
        event.stopPropagation?.()
        if (event.eventType !== "release" && event.eventType !== "repeat") setFocus({ window: 0, control: 0 })
        return true
      }
      event.preventDefault?.()
      event.stopPropagation?.()
      if (event.eventType === "release" || event.eventType === "repeat") return true
      if (event.name === "escape" || event.name === SUDO_BAND_KEY) setFocus(null)
      else if (event.name === "left" || event.name === "right") setFocus({ ...focus, control: (focus.control + 1) % controls.length })
      else if (event.name === "up") setFocus({ ...focus, window: (focus.window + windows.length - 1) % windows.length })
      else if (event.name === "down") setFocus({ ...focus, window: (focus.window + 1) % windows.length })
      else if (event.name === "return" || event.name === "enter") {
        const window = windows[focus.window]
        const action = controls[focus.control]![1]
        setFocus(null)
        if (window) action(window)
      }
      return true
    },
  }
}
