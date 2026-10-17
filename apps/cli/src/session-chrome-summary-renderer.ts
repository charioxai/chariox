import {
  BoxRenderable,
  type CliRenderer,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type { FooterFlash } from "./footer-flash-controller.js"

import { theme } from "./theme.js"

export type SessionChromeSummaryRenderState = {
  promptStateText?: TextRenderable
  footerSummaryText?: TextRenderable
  footerFlashText?: TextRenderable
  toastBox?: BoxRenderable
  toastText?: TextRenderable
}

type SessionChromeSummaryRenderOptions = {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  state: SessionChromeSummaryRenderState
  promptStateBox: BoxRenderable | undefined
  footerSummaryBox: BoxRenderable | undefined
  promptStateLabel: string
  promptStateTone: "error" | "thinking" | "muted"
  footerSummary: string
  footerFlash: FooterFlash | null
}

export function createSessionChromeSummaryRenderState(): SessionChromeSummaryRenderState {
  return {}
}

export function renderSessionChromeSummary(options: SessionChromeSummaryRenderOptions): void {
  if (options.promptStateBox?.isDestroyed || options.footerSummaryBox?.isDestroyed) return
  ensureSessionChromeSummaryRenderables(options)
  setTextRenderable(
    options.state.promptStateText,
    options.promptStateLabel,
    options.promptStateTone === "error"
      ? theme.error
      : options.promptStateTone === "thinking"
        ? theme.primary
        : theme.textMuted,
  )
  options.promptStateBox?.requestRender()
  setTextRenderable(options.state.footerSummaryText, options.footerSummary, theme.textMuted)
  setTextRenderable(
    options.state.footerFlashText,
    options.footerFlash && options.footerFlash.display !== "toast" ? ` • ${options.footerFlash.message}` : "",
    options.footerFlash?.tone === "error" ? theme.error : theme.info,
    options.footerFlash ? TextAttributes.BOLD : TextAttributes.NONE,
  )
  // MP-08/MP-11: client command results float outside the agent transcript.
  const toast = options.footerFlash?.display === "toast" ? options.footerFlash : null
  if (toast && !options.state.toastBox) {
    const box = new BoxRenderable(options.renderer, { position: "absolute", left: 2, right: 2, bottom: 4, zIndex: 1000, border: true, padding: 1, backgroundColor: theme.backgroundPanel })
    options.state.toastText = new TextRenderable(options.renderer, { wrapMode: "word" })
    box.add(options.state.toastText)
    const renderer = options.renderer as CliRenderer
    renderer.root.add(box)
    options.state.toastBox = box
  }
  if (options.state.toastBox) {
    options.state.toastBox.visible = Boolean(toast)
    setTextRenderable(options.state.toastText, toast?.message ?? "", toast?.tone === "error" ? theme.error : theme.info)
    options.state.toastBox.requestRender()
  }
  options.footerSummaryBox?.requestRender()
}

function ensureSessionChromeSummaryRenderables(options: SessionChromeSummaryRenderOptions): void {
  if (options.promptStateBox && !options.state.promptStateText) {
    options.state.promptStateText = new TextRenderable(options.renderer, { fg: theme.textMuted, wrapMode: "none" })
    options.promptStateBox.add(options.state.promptStateText)
  }
  if (options.footerSummaryBox && !options.state.footerSummaryText) {
    options.state.footerSummaryText = new TextRenderable(options.renderer, { fg: theme.textMuted, wrapMode: "none" })
    options.state.footerFlashText = new TextRenderable(options.renderer, { fg: theme.info, wrapMode: "none" })
    options.footerSummaryBox.add(options.state.footerSummaryText)
    options.footerSummaryBox.add(options.state.footerFlashText)
  }
}

function setTextRenderable(
  text: TextRenderable | undefined,
  content: string,
  fg: (typeof theme)[keyof typeof theme],
  attributes = TextAttributes.NONE,
): void {
  if (!text || text.isDestroyed) {
    return
  }
  text.content = content
  text.fg = fg
  text.attributes = attributes
}
