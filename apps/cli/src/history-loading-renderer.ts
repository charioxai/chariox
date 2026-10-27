import {
  TextRenderable,
  type BoxRenderable,
} from "@opentui/core"

import { theme } from "./theme.js"

type RenderHistoryLoadingOptions = {
  box: BoxRenderable | undefined
  text: TextRenderable | undefined
  loading: boolean
  message: string | null
  renderer: ConstructorParameters<typeof TextRenderable>[0]
  assignText: (text: TextRenderable | undefined) => void
}

export function renderHistoryLoadingIndicator({
  box,
  text,
  loading,
  message,
  renderer,
  assignText,
}: RenderHistoryLoadingOptions): void {
  if (!box) {
    return
  }
  const content = message ?? "loading..."
  const visible = loading || Boolean(message)
  box.visible = visible
  if (visible) {
    if (!text) {
      const nextText = new TextRenderable(renderer, {
        content,
        fg: theme.textMuted,
        wrapMode: "none",
      })
      box.add(nextText)
      assignText(nextText)
    } else {
      text.content = content
    }
  } else if (text) {
    box.remove(text.id)
    text.destroyRecursively()
    assignText(undefined)
  }
  box.requestRender()
}
