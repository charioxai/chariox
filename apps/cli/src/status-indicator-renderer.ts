import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type { SessionFocusedStatusBadge } from "@chariox/kernel-client/session-runtime-status"
import { renderStatusBadgeParts } from "./status-badge-renderer.js"
import { theme } from "./theme.js"

export type StatusIndicatorRenderState = {
  openText?: TextRenderable
  closeText?: TextRenderable
  labelTexts: TextRenderable[]
}

type StatusIndicatorRenderOptions = {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  box: BoxRenderable | undefined
  state: StatusIndicatorRenderState
  attached: boolean
  badge: SessionFocusedStatusBadge | null
  badgeWidth: number
  animationFrame: number
}

export function createStatusIndicatorRenderState(): StatusIndicatorRenderState {
  return {
    labelTexts: [],
  }
}

export function renderStatusIndicator(options: StatusIndicatorRenderOptions): void {
  ensureStatusIndicatorRenderables(options)
  if (!options.attached || !options.badge) {
    setTextRenderable(options.state.openText, "", theme.textMuted)
    ensureStatusLabelTextCount(options, options.badgeWidth)
    renderStatusBadgeParts(options.state.labelTexts, [], options.badgeWidth, options.animationFrame)
    setTextRenderable(options.state.closeText, "", theme.textMuted)
    options.box?.requestRender()
    return
  }

  const width = Math.max(options.badgeWidth, options.badge.label.length)
  setTextRenderable(options.state.openText, "", theme.textMuted)
  ensureStatusLabelTextCount(options, width)
  renderStatusBadgeParts(options.state.labelTexts, options.badge.parts, width, options.animationFrame)
  setTextRenderable(options.state.closeText, "", theme.textMuted)
  options.box?.requestRender()
}

function ensureStatusIndicatorRenderables(options: StatusIndicatorRenderOptions): void {
  if (!options.box || options.state.openText) {
    return
  }
  options.state.openText = new TextRenderable(options.renderer, { content: "", fg: theme.textMuted, wrapMode: "none" })
  options.state.closeText = new TextRenderable(options.renderer, { content: "", fg: theme.textMuted, wrapMode: "none" })
  options.box.add(options.state.openText)
  options.state.labelTexts = Array.from({ length: options.badgeWidth }, () => {
    const text = new TextRenderable(options.renderer, { wrapMode: "none" })
    options.box!.add(text)
    return text
  })
  options.box.add(options.state.closeText)
}

function ensureStatusLabelTextCount(options: StatusIndicatorRenderOptions, count: number): void {
  if (!options.box) {
    return
  }
  const moveCloseText = Boolean(options.state.closeText && options.state.labelTexts.length < count)
  if (options.state.closeText && moveCloseText) {
    options.box.remove(options.state.closeText.id)
  }
  while (options.state.labelTexts.length < count) {
    const text = new TextRenderable(options.renderer, { wrapMode: "none" })
    options.state.labelTexts.push(text)
    options.box.add(text)
  }
  if (options.state.closeText && moveCloseText) {
    options.box.add(options.state.closeText)
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
