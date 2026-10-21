import { BoxRenderable, MouseButton, TextAttributes, TextRenderable, type CliRenderer } from "@opentui/core"
import type { KernelSudoTurn } from "@chariox/kernel-client/kernel-types"
import { sudoWindowLine } from "./sudo-window-line.js"
import { theme } from "./theme.js"

export function createSudoWindowBand(renderer: CliRenderer, actions: {
  extend(window: KernelSudoTurn): void
  revoke(window: KernelSudoTurn): void
}) {
  let band: BoxRenderable | undefined
  let lastFrame = ""
  return {
    assign(value: BoxRenderable) { band = value; lastFrame = "" },
    render(windows: readonly KernelSudoTurn[], now: number, agentLabel: (agentId: string) => string) {
      if (!band) return
      const lines = windows.map((window) => sudoWindowLine(window, now, agentLabel(window.agent_id)))
      // MP-08/MP-10/MP-11 P3: handlers capture the window, so its revision
      // must invalidate the frame even when the rounded deadline is unchanged.
      const frame = JSON.stringify([lines, windows.map((window) => window.revision), theme.warning, theme.backgroundElement])
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
        for (const [label, action] of [["[Extend]", actions.extend], ["[Revoke]", actions.revoke]] as const) {
          const control = new TextRenderable(renderer, { content: label, fg: theme.primary, flexShrink: 0 })
          control.onMouseUp = (event) => {
            event.stopPropagation()
            if (event.button === MouseButton.LEFT) action(window)
          }
          row.add(control)
        }
        band!.add(row)
      })
      band.requestRender()
    },
  }
}
