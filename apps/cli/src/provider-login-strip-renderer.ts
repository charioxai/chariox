import { BoxRenderable, MouseButton, StyledText, TextAttributes, TextRenderable, link, underline } from "@opentui/core"
import { setTimeout as startTimeout } from "node:timers"

import type { RuntimeInteraction } from "./cli-types.js"
import { renderInteractionCustomChoiceValue } from "./interaction-custom-choice-render.js"
import type { ProviderLoginStripState } from "./provider-login-interaction-controller.js"
import { theme } from "./theme.js"

export type ProviderLoginStripOptions = {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  container: BoxRenderable
  interaction: RuntimeInteraction
  state: ProviderLoginStripState
  focused: boolean
  selectedIndex: number
  reply: string
  editing: boolean
  now: number
  onLinkClick: () => void
}

export function formatProviderLoginCountdown(deadlineMs: number | null, now: number): string {
  if (deadlineMs === null) return ""
  const seconds = Math.max(0, Math.ceil((deadlineMs - now) / 1_000))
  return `expires in ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`
}

/** MP-08/MP-11: numbered steps, the link alone on its rows (wrapped only at
 * the edge, never with added characters, and an OSC 8 hyperlink where the
 * terminal supports it), a countdown and the code field. */
export function renderProviderLoginStrip(options: ProviderLoginStripOptions): void {
  const { renderer, container, interaction, state, focused } = options
  const text = (content: string | StyledText, fg = theme.text, attributes: number = TextAttributes.NONE) =>
    new TextRenderable(renderer, { content, fg, attributes, wrapMode: "word" })

  const header = new BoxRenderable(renderer, { flexDirection: "row", gap: 1, flexShrink: 0 })
  const title = text(state.view.title, theme.warning, TextAttributes.BOLD)
  title.flexGrow = 1
  header.add(title)
  const countdown = new TextRenderable(renderer, {
    content: formatProviderLoginCountdown(state.deadlineMs, options.now),
    fg: theme.textMuted,
    wrapMode: "none",
  })
  countdown.flexShrink = 0
  header.add(countdown)
  container.add(header)

  const url = state.view.url
  if (!url && state.problem) {
    // The provider dropped its link after rejecting a code.
    container.add(text("1. This link expired. Press Esc, then 1 to cancel, and sign in again.", theme.warning))
  } else if (!url) {
    container.add(text("1. Waiting for the authorization link…", theme.textMuted))
  } else {
    container.add(text("1. Open the link (click it):"))
    container.add(linkRows(options, url))
  }
  state.view.steps.forEach((step, index) => container.add(text(`${index + 2}. ${step}`)))
  if (state.problem) container.add(text(state.problem, theme.error))

  if (state.checking) {
    container.add(text("Checking the code…", theme.info))
    return
  }
  const custom = interaction.custom_choice
  const row = new BoxRenderable(renderer, { flexDirection: "row", gap: 2, flexShrink: 0 })
  if (state.view.takesCode && custom) {
    const selected = focused && options.selectedIndex === interaction.choices.length
    const value = renderInteractionCustomChoiceValue(options.reply, "paste the code", custom.input_kind)
    const field = new TextRenderable(renderer, {
      content: `${selected ? ">" : " "} Code: ${value}${options.editing ? "▏" : ""}`,
      fg: selected ? theme.background : theme.primary,
      ...(selected ? { bg: theme.primary } : {}),
      attributes: selected ? TextAttributes.BOLD : TextAttributes.NONE,
      wrapMode: "none",
    })
    field.flexShrink = 0
    row.add(field)
  }
  interaction.choices.forEach((choice, index) => {
    const selected = focused && options.selectedIndex === index
    const choiceText = new TextRenderable(renderer, {
      content: `${selected ? ">" : " "} ${index + 1}.${choice.label}`,
      fg: selected ? theme.background : theme.textMuted,
      ...(selected ? { bg: theme.textMuted } : {}),
      wrapMode: "none",
    })
    choiceText.flexShrink = 0
    row.add(choiceText)
  })
  const hints = options.reply || !url
    ? "Enter sends · Esc, then 1 cancels"
    : "O opens the link · C copies it · Esc, then 1 cancels"
  const hint = new TextRenderable(renderer, { content: hints, fg: theme.textMuted, wrapMode: "word" })
  hint.flexShrink = 1
  row.add(hint)
  container.add(row)
}

function linkRows(options: ProviderLoginStripOptions, url: string): TextRenderable {
  const rows = new TextRenderable(options.renderer, {
    content: new StyledText([link(url)(underline(url))]),
    fg: theme.primary,
    wrapMode: "char",
  })
  rows.onMouseUp = (event) => {
    if (event.button !== MouseButton.LEFT) return
    event.stopPropagation()
    startTimeout(options.onLinkClick, 0)
  }
  return rows
}
