import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import { theme } from "./theme.js"

export type SessionChromeSummaryRenderState = {
  promptStateText?: TextRenderable
  footerSummaryText?: TextRenderable
  footerFlashText?: TextRenderable
}

type FooterFlash = {
  message: string
  tone: "info" | "error"
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
    options.footerFlash ? ` • ${options.footerFlash.message}` : "",
    options.footerFlash?.tone === "error" ? theme.error : theme.info,
    options.footerFlash ? TextAttributes.BOLD : TextAttributes.NONE,
  )
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
  if (!text) {
    return
  }
  text.content = content
  text.fg = fg
  text.attributes = attributes
}
