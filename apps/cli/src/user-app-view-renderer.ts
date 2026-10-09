import { BoxRenderable, ScrollBoxRenderable, TextRenderable, type CliRenderer } from "@opentui/core"
import { theme } from "./theme.js"
import type { UserAppViewPanel } from "./user-app-view-controller.js"

export function createUserAppViewRenderer(renderer: CliRenderer) {
  const box = new BoxRenderable(renderer, { id: "user-app-view-prototype", position: "absolute", top: 1, left: 1,
    border: true, flexDirection: "column", padding: 1, visible: false, zIndex: 50 })
  const body = new ScrollBoxRenderable(renderer, { flexGrow: 1, scrollY: true, scrollX: false })
  const text = new TextRenderable(renderer, { content: "", wrapMode: "word", flexShrink: 0 })
  const footer = new TextRenderable(renderer, { content: "Tab / typing / Enter: App · PgUp/PgDn: outline · Esc: prompt · Ctrl+W: close view · F8: passkey", wrapMode: "word", flexShrink: 0 })
  body.add(text); box.add(body); box.add(footer); renderer.root.add(box)
  return {
    scroll(direction: -1 | 1) { body.scrollBy(direction * 4) },
    render(view: UserAppViewPanel, dimensions: {width: number; height: number}) {
      box.visible = view.open
      box.width = Math.max(1, dimensions.width - 2); box.height = Math.max(1, dimensions.height - 2)
      box.backgroundColor = theme.backgroundPanel; box.borderColor = theme.primary
      text.fg = theme.text; footer.fg = theme.textMuted; text.content = view.text
      box.requestRender()
    },
    dispose() { renderer.root.remove(String(box.id)); box.destroyRecursively() },
  }
}
