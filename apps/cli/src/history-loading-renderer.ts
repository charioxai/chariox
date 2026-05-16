import {
  TextRenderable,
  type BoxRenderable,
} from "@opentui/core"

import { theme } from "./theme.js"

type RenderHistoryLoadingOptions = {
  box: BoxRenderable | undefined
  text: TextRenderable | undefined
  loading: boolean
  renderer: ConstructorParameters<typeof TextRenderable>[0]
  assignText: (text: TextRenderable | undefined) => void
}

export function renderHistoryLoadingIndicator({
  box,
  text,
  loading,
  renderer,
  assignText,
}: RenderHistoryLoadingOptions): void {
  if (!box) {
    return
  }
  box.visible = loading
  if (loading) {
    if (!text) {
      const nextText = new TextRenderable(renderer, {
        content: "loading...",
        fg: theme.textMuted,
        wrapMode: "none",
      })
      box.add(nextText)
      assignText(nextText)
    }
  } else if (text) {
    box.remove(text.id)
    text.destroyRecursively()
    assignText(undefined)
  }
  box.requestRender()
}
