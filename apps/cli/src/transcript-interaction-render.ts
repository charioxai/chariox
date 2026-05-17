import { setTimeout as startTimeout } from "node:timers"

import {
  BoxRenderable,
  MouseButton,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type { TranscriptEntry } from "./cli-types.js"
import { theme } from "./theme.js"
import { transcriptTextColor } from "./transcript-render-theme.js"
import { applyTranscriptTextContent } from "./transcript-text-render.js"

type RenderContext = ConstructorParameters<typeof BoxRenderable>[0]

export function buildTurnToggleContent(
  renderer: RenderContext,
  body: BoxRenderable,
  entry: TranscriptEntry,
  onToggleTurn: (turnId: number | null | undefined, toggleEntryId?: number) => void,
) {
  const text = new TextRenderable(renderer, {
    fg: transcriptTextColor(entry),
    wrapMode: "word",
  })
  text.onMouseUp = (event) => {
    if (event.button !== MouseButton.LEFT) {
      return
    }
    event.stopPropagation()
    startTimeout(() => {
      onToggleTurn(entry.turnId, entry.id)
    }, 0)
  }
  applyTranscriptTextContent(text, entry)
  body.add(text)
}

export function buildCollapsedTranscriptBlob(
  renderer: RenderContext,
  body: BoxRenderable,
  entry: TranscriptEntry,
  onToggleBlob: (entryId: number, collapsed: boolean) => void,
) {
  body.add(
    new TextRenderable(renderer, {
      content: [entry.blobTitle, entry.blobSummary].filter(Boolean).join("  "),
      fg: transcriptTextColor(entry),
      wrapMode: "word",
      attributes: TextAttributes.BOLD,
    }),
  )
  body.add(buildBlobToggleLabel(renderer, "click to expand", () => onToggleBlob(entry.id, false)))
}

export function buildBlobToggleLabel(renderer: RenderContext, content: string, onClick: () => void) {
  const text = new TextRenderable(renderer, {
    content,
    fg: theme.textMuted,
    wrapMode: "word",
  })
  text.onMouseUp = (event) => {
    if (event.button !== MouseButton.LEFT) {
      return
    }
    event.stopPropagation()
    startTimeout(() => {
      onClick()
    }, 0)
  }
  return text
}
