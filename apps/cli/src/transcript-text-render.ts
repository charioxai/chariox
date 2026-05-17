import {
  RGBA,
  TextAttributes,
  TextNodeRenderable,
  type TextRenderable,
} from "@opentui/core"

import type { TranscriptEntry } from "./cli-types.js"
import { theme } from "./theme.js"
import { splitInlineCodeSpans } from "./transcript.js"
import { transcriptInlineCodeColor } from "./transcript-render-theme.js"

export function applyTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  text.clear()
  if (entry.role === "tool") {
    applyToolTranscriptTextContent(text, entry)
    return
  }
  if (entry.role === "user") {
    applyPromptTranscriptTextContent(text, entry)
    return
  }
  appendTranscriptSpans(text, entry, entry.text)
}

function applyPromptTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  const lines = entry.text.split("\n")
  for (const [lineIndex, line] of lines.entries()) {
    appendPromptTranscriptLine(text, entry, line)
    if (lineIndex < lines.length - 1) {
      text.add("\n")
    }
  }
}

function appendPromptTranscriptLine(text: TextRenderable, entry: TranscriptEntry, line: string) {
  const matches = Array.from(line.matchAll(/\[(image|pdf|file)\s+(\d+)\]/gi))
  if (matches.length === 0) {
    appendTranscriptSpans(text, entry, line)
    return
  }
  let offset = 0
  for (const match of matches) {
    const index = match.index ?? 0
    if (index > offset) {
      appendTranscriptSpans(text, entry, line.slice(offset, index))
    }
    appendAttachmentChip(
      text,
      tokenMime(match[1] ?? "file"),
      `[${(match[1] ?? "file").toLowerCase()} ${match[2] ?? "1"}]`,
    )
    offset = index + match[0].length
  }
  if (offset < line.length) {
    appendTranscriptSpans(text, entry, line.slice(offset))
  }
}

function appendAttachmentChip(text: TextRenderable, mime: string, filename: string) {
  const label = mime.startsWith("image/") ? "img" : mime === "application/pdf" ? "pdf" : "txt"
  const colors = mime.startsWith("image/")
    ? { accentBg: RGBA.fromHex("#f0d77d"), accentFg: RGBA.fromHex("#1f1400"), bodyBg: RGBA.fromHex("#2e2615") }
    : mime === "application/pdf"
      ? { accentBg: RGBA.fromHex("#8cc0ff"), accentFg: RGBA.fromHex("#09182b"), bodyBg: RGBA.fromHex("#172534") }
      : { accentBg: RGBA.fromHex("#8fd8a8"), accentFg: RGBA.fromHex("#0d1f13"), bodyBg: RGBA.fromHex("#173022") }
  text.add(TextNodeRenderable.fromString(` ${label} `, {
    fg: colors.accentFg,
    bg: colors.accentBg,
    attributes: TextAttributes.BOLD,
  }))
  text.add(TextNodeRenderable.fromString(` ${filename} `, {
    fg: theme.text,
    bg: colors.bodyBg,
    attributes: TextAttributes.BOLD,
  }))
}

function tokenMime(kind: string) {
  const value = kind.toLowerCase()
  if (value === "image") {
    return "image/png"
  }
  if (value === "pdf") {
    return "application/pdf"
  }
  return "text/plain"
}

function appendTranscriptSpans(text: TextRenderable, entry: TranscriptEntry, value: string) {
  for (const span of splitInlineCodeSpans(value)) {
    text.add(
      TextNodeRenderable.fromString(
        span.text,
        span.code
          ? {
              fg: transcriptInlineCodeColor(entry),
              attributes: TextAttributes.BOLD,
            }
          : undefined,
      ),
    )
  }
}

function applyToolTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
  const newlineIndex = entry.text.indexOf("\n")
  const title = newlineIndex === -1 ? entry.text : entry.text.slice(0, newlineIndex)
  const rest = newlineIndex === -1 ? "" : entry.text.slice(newlineIndex)

  if (title) {
    text.add(TextNodeRenderable.fromString(title, { fg: theme.secondary }))
  }
  for (const span of splitInlineCodeSpans(rest)) {
    text.add(
      TextNodeRenderable.fromString(
        span.text,
        span.code
          ? {
              fg: transcriptInlineCodeColor(entry),
              attributes: TextAttributes.BOLD,
            }
          : {
              fg: theme.text,
            },
      ),
    )
  }
}
