import { BoxRenderable, ScrollBoxRenderable, TextRenderable, MouseButton, type CliRenderer } from "@opentui/core"
import type { KernelApprovalView } from "./kernel-approval-controller.js"
import { theme } from "./theme.js"

export function createKernelApprovalRenderer(renderer: CliRenderer, actions: {
  show(): void
  choose(interactionId: string, choiceId: string): void
}) {
  let box: BoxRenderable | undefined
  let body: ScrollBoxRenderable | undefined
  let lastFrame = ""
  return {
    assign(value: BoxRenderable) { box = value; lastFrame = "" },
    scroll(direction: -1 | 1) { body?.scrollBy(direction * 4) },
    render(view: KernelApprovalView, dimensions: { width: number; height: number }) {
      if (!box) return
      const frame = JSON.stringify([view, dimensions, theme.text, theme.primary, theme.backgroundPanel, theme.backgroundElement])
      if (lastFrame === frame) return
      lastFrame = frame
      body = undefined
      for (const child of [...box.getChildren()]) {
        box.remove(String(child.id))
        child.destroyRecursively()
      }
      box.visible = view.count > 0
      if (!view.count) return
      box.width = dimensions.width
      box.height = view.open ? dimensions.height : 1
      const text = (parent: BoxRenderable, content: string, accent = false) => {
        const node = new TextRenderable(renderer, {
          content, wrapMode: "word", fg: accent ? theme.primary : theme.text,
          flexShrink: 0,
        })
        parent.add(node)
        return node
      }
      const indicator = new BoxRenderable(renderer, {
        position: "absolute", right: 0, top: 0, height: 1,
        backgroundColor: theme.backgroundElement, paddingLeft: 1, paddingRight: 1,
      })
      text(indicator, `Chariox · ${view.count} approval${view.count === 1 ? "" : "s"} · F8`, true)
      indicator.onMouseUp = (event) => {
        event.stopPropagation()
        if (event.button === MouseButton.LEFT) actions.show()
      }
      box.add(indicator)
      if (!view.open || !view.interaction) return
      const panel = new BoxRenderable(renderer, {
        position: "absolute", top: Math.min(2, Math.max(0, dimensions.height - 5)),
        left: Math.max(0, Math.floor((dimensions.width - 76) / 2)),
        width: Math.min(76, dimensions.width), height: Math.max(4, dimensions.height - 4),
        border: true, borderColor: theme.primary, backgroundColor: theme.backgroundPanel,
        paddingLeft: 1, paddingRight: 1, flexDirection: "column",
      })
      panel.onMouseUp = (event) => event.stopPropagation()
      text(panel, `Chariox approval ${view.index + 1} of ${view.count} · Esc to dismiss`, true)
      body = new ScrollBoxRenderable(renderer, {
        flexGrow: 1, flexShrink: 1, scrollY: true, scrollX: false,
      })
      panel.add(body)
      text(body, view.interaction.title || "Kernel approval")
      text(body, view.interaction.message)
      view.interaction.choices.forEach((choice, index) => {
        const row = new BoxRenderable(renderer, {
          flexShrink: 0, backgroundColor: view.selected === index ? theme.backgroundElement : theme.backgroundPanel,
        })
        text(row, `${view.selected === index ? "›" : " "} ${choice.label}`, view.selected === index)
        row.onMouseUp = (event) => {
          event.stopPropagation()
          if (event.button === MouseButton.LEFT && view.connected && !view.pending) actions.choose(view.interaction!.id, choice.id)
        }
        body!.add(row)
      })
      if (view.error) text(body, view.error)
      if (view.selected !== null && !view.pending) {
        const choice = view.interaction.choices[view.selected]
        if (choice) text(panel, `Selected: ${choice.label}`, true)
      }
      text(panel, view.pending ? "Waiting for kernel confirmation…"
        : !view.connected ? "Disconnected · reconnect to respond"
        : "↑/↓ select · Enter confirm · ←/→ approvals · PgUp/PgDn scroll")
      box.add(panel)
      box.requestRender()
    },
  }
}
