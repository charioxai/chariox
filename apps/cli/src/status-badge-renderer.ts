import {
  TextAttributes,
  type TextRenderable,
} from "@opentui/core"

import type { SessionStatusBadgePart } from "@chariox/kernel-client/session-runtime-status"
import type { StatusBadgeTone } from "./split-pane-footer.js"
import { theme } from "./theme.js"

const applied = new WeakMap<TextRenderable, { character: string; fg: unknown; attributes: number }>()

export function renderStatusBadgeLabel(
  texts: TextRenderable[],
  label: string,
  tone: StatusBadgeTone,
  minWidth: number,
  animationFrame: number,
): void {
  renderStatusBadgeParts(texts, [{ label, tone }], minWidth, animationFrame)
}

export function renderStatusBadgeParts(
  texts: TextRenderable[],
  parts: SessionStatusBadgePart[],
  minWidth: number,
  animationFrame: number,
): void {
  const cells = badgeCells(parts)
  const width = Math.max(minWidth, cells.length)
  for (let index = 0; index < texts.length; index += 1) {
    const cell = index < width ? cells[index] : undefined
    const character = cell?.character ?? " "
    const tone = cell?.tone ?? "idle"
    let fg = theme.success
    if (tone === "disconnected" || tone === "error") {
      fg = theme.error
    } else if (tone === "working") {
      const distance = reflectedDistance(cell?.partIndex ?? 0, cell?.partLength ?? 0, animationFrame)
      fg = distance === 0 ? theme.primary : distance === 1 ? theme.warning : theme.secondary
    }
    const text = texts[index]
    if (!text) {
      continue
    }
    const attributes = tone === "working" && character.trim() ? TextAttributes.BOLD : TextAttributes.NONE
    // Every assignment rebuilds the native text buffer and startup repaints the
    // same badge many times, so write a cell only when something changed. A
    // blank cell shows nothing whatever its colour: only its content matters.
    if (!character.trim()) {
      if (applied.get(text)?.character !== character) text.content = character
      applied.set(text, { character, fg: undefined, attributes: -1 })
      continue
    }
    const last = applied.get(text)
    if (last && last.character === character && last.fg === fg && last.attributes === attributes) continue
    applied.set(text, { character, fg, attributes })
    text.content = character
    text.fg = fg
    text.attributes = attributes
  }
}

function badgeCells(parts: SessionStatusBadgePart[]): Array<{
  character: string
  tone: StatusBadgeTone
  partIndex: number
  partLength: number
}> {
  const cells: Array<{
    character: string
    tone: StatusBadgeTone
    partIndex: number
    partLength: number
  }> = []
  for (const [partOffset, part] of parts.entries()) {
    if (partOffset > 0) {
      cells.push({
        character: " ",
        tone: "idle",
        partIndex: 0,
        partLength: 0,
      })
    }
    for (let index = 0; index < part.label.length; index += 1) {
      cells.push({
        character: part.label[index] ?? " ",
        tone: part.tone,
        partIndex: index,
        partLength: part.label.length,
      })
    }
  }
  return cells
}

function reflectedDistance(index: number, length: number, frame: number): number {
  if (length <= 1) {
    return 0
  }
  const span = length - 1
  const cycle = span * 2
  const position = frame % cycle
  const highlight = position <= span ? position : cycle - position
  return Math.abs(index - highlight)
}
