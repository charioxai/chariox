import { BoxRenderable, TextRenderable, type CliRenderer } from "@opentui/core"
import { readHiddenInput } from "@chariox/kernel-client/hidden-input"
import { theme } from "./theme.js"

/** A human-only masked surface; no textarea, draft, transcript or history. */
export function createCliSecretInput(renderer: CliRenderer) {
  let pending: AbortController | null = null
  return {
    cancel() { pending?.abort() },
    async readSecret(prompt: string): Promise<string> {
      if (pending) throw new Error("secret input is already open")
      const abort = new AbortController()
      pending = abort
      const focus = renderer.currentFocusedRenderable
      const kittyKeyboard = renderer.useKittyKeyboard
      let panel: BoxRenderable | undefined
      try {
        focus?.blur()
        if (kittyKeyboard) renderer.disableKittyKeyboard()
        panel = new BoxRenderable(renderer, {
          id: "secret-input",
          position: "absolute", top: "30%", left: "10%", width: "80%",
          zIndex: 10000, backgroundColor: theme.backgroundPanel,
          padding: 2, flexDirection: "column", gap: 1,
        })
        panel.add(new TextRenderable(renderer, { content: prompt, fg: theme.text }))
        const mask = new TextRenderable(renderer, { content: "", fg: theme.primary })
        panel.add(mask)
        panel.add(new TextRenderable(renderer, {
          content: "Enter submits • Ctrl-C cancels", fg: theme.textMuted,
        }))
        renderer.root.add(panel)
        return await readHiddenInput({
          input: process.stdin, output: process.stdout, signal: abort.signal,
          renderMask: (value) => { mask.content = value; renderer.requestRender() },
        })
      } finally {
        pending = null
        if (kittyKeyboard) renderer.enableKittyKeyboard()
        if (panel) {
          renderer.root.remove(panel.id)
          panel.destroyRecursively()
        }
        if (focus && !focus.isDestroyed) focus.focus()
        renderer.requestRender()
      }
    },
  }
}
