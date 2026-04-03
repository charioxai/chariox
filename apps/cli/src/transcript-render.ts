import { setTimeout as startTimeout } from "node:timers"

import {
  BoxRenderable,
  DiffRenderable,
  MarkdownRenderable,
  MouseButton,
  RGBA,
  TextAttributes,
  TextNodeRenderable,
  TextRenderable,
  type SyntaxStyle,
} from "@opentui/core"

import type { TranscriptEntry } from "./cli-types.js"
import { SplitBorder, theme } from "./theme.js"
import {
  guessPathFenceLanguage,
  normalizeMarkdownFenceInfoStrings,
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  shouldRenderTranscriptAsMarkdown,
  splitInlineCodeSpans,
} from "./transcript.js"

type RenderContext = ConstructorParameters<typeof BoxRenderable>[0]

export type TranscriptEntryRenderable = {
  entry: TranscriptEntry
  wrapper: BoxRenderable
  update: (entry: TranscriptEntry) => void
}

export type TranscriptSurfaceTone = "default" | "focused" | "faded"

export function buildTranscriptEntryRenderable(
  renderer: RenderContext,
  entry: TranscriptEntry,
  transcriptSyntax: SyntaxStyle,
  onToggleTurn: (turnId: number | null | undefined, toggleEntryId?: number) => void,
  surfaceTone: TranscriptSurfaceTone = "default",
) {
  const patch = readTranscriptApplyPatch(entry)
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 1,
    flexDirection: "column",
  })
  const bodyColor = transcriptBodyColor(entry, surfaceTone)
  const body = new BoxRenderable(renderer, {
    paddingLeft: 1,
    paddingRight: 0,
    paddingTop: 1,
    paddingBottom: 1,
    ...(bodyColor ? { backgroundColor: bodyColor } : {}),
  })
  let update: (nextEntry: TranscriptEntry) => void

  if (patch) {
    buildApplyPatchTranscriptContent(renderer, body, patch, transcriptSyntax, surfaceTone)
    update = (nextEntry) => {
      for (const child of body.getChildren()) {
        body.remove(child.id)
        child.destroyRecursively()
      }
      const nextPatch = readTranscriptApplyPatch(nextEntry)
      if (nextPatch) {
        buildApplyPatchTranscriptContent(renderer, body, nextPatch, transcriptSyntax, surfaceTone)
        return
      }
      const markdown = new MarkdownRenderable(renderer, {
        content: normalizeMarkdownFenceInfoStrings(nextEntry.text),
        syntaxStyle: transcriptSyntax,
        conceal: true,
        concealCode: false,
        streaming: true,
      })
      body.add(markdown)
      markdown.requestRender()
    }
  } else if (shouldRenderTranscriptAsMarkdown(entry.role, entry.text)) {
    const markdown = new MarkdownRenderable(renderer, {
      content: normalizeMarkdownFenceInfoStrings(entry.text),
      syntaxStyle: transcriptSyntax,
      conceal: true,
      concealCode: false,
      streaming: true,
    })
    body.add(markdown)
    update = (nextEntry) => {
      markdown.content = normalizeMarkdownFenceInfoStrings(nextEntry.text)
      markdown.streaming = true
      markdown.requestRender()
    }
  } else {
    const text = new TextRenderable(renderer, {
      fg: transcriptTextColor(entry),
      wrapMode: "word",
    })
    if (entry.role === "turn_toggle") {
      text.onMouseUp = (event) => {
        if (event.button !== MouseButton.LEFT) {
          return
        }
        event.stopPropagation()
        startTimeout(() => {
          onToggleTurn(entry.turnId, entry.id)
        }, 0)
      }
    }
    applyTranscriptTextContent(text, entry)
    body.add(text)
    update = (nextEntry) => {
      applyTranscriptTextContent(text, nextEntry)
    }
  }

  if (transcriptUsesAccentBorder(entry)) {
    const border = new BoxRenderable(renderer, {
      border: ["left"],
      customBorderChars: SplitBorder.customBorderChars,
      borderColor: transcriptAccent(entry),
    })
    border.add(body)
    wrapper.add(border)
  } else {
    wrapper.add(body)
  }

  return { entry, wrapper, update }
}

export function transcriptRenderMode(entry: TranscriptEntry) {
  if (readTranscriptApplyPatch(entry)) {
    return "patch"
  }
  if (shouldRenderTranscriptAsMarkdown(entry.role === "turn_summary" ? "assistant" : entry.role, entry.text)) {
    return "markdown"
  }
  return "text"
}

export function resolveTranscriptSurfaceTone(splitActive: boolean, focused: boolean): TranscriptSurfaceTone {
  if (!splitActive) {
    return "default"
  }
  return focused ? "focused" : "faded"
}

export function transcriptSurfacePalette(surfaceTone: TranscriptSurfaceTone) {
  if (surfaceTone === "focused") {
    return {
      panel: theme.backgroundPanel,
      element: theme.backgroundElement,
    }
  }
  if (surfaceTone === "faded") {
    return {
      panel: RGBA.fromHex("#171717"),
      element: RGBA.fromHex("#202020"),
    }
  }
  return {
    panel: theme.backgroundPanel,
    element: theme.backgroundElement,
  }
}

export function renderPromptTranscript(prompt: string) {
  const text = prompt.trimEnd()
  return text ? `${text}\n` : ""
}

function readTranscriptApplyPatch(entry: TranscriptEntry) {
  const parsed = parseToolTranscriptUpdate(entry.sourceText ?? entry.text)
  if (!parsed) {
    return null
  }
  const files = readApplyPatchFiles(parsed)
  return files.length > 0 ? files : null
}

function buildApplyPatchTranscriptContent(
  renderer: RenderContext,
  body: BoxRenderable,
  files: ReturnType<typeof readApplyPatchFiles>,
  transcriptSyntax: SyntaxStyle,
  surfaceTone: TranscriptSurfaceTone,
) {
  const palette = transcriptSurfacePalette(surfaceTone)
  body.flexDirection = "column"
  body.gap = 1
  body.add(
    new TextRenderable(renderer, {
      content: `patch · ${files.length} ${files.length === 1 ? "file" : "files"}`,
      fg: theme.secondary,
      attributes: TextAttributes.BOLD,
      wrapMode: "word",
    }),
  )

  for (const file of files) {
    const block = new BoxRenderable(renderer, {
      flexDirection: "column",
      border: ["left"],
      customBorderChars: SplitBorder.customBorderChars,
      borderColor: theme.borderSubtle,
      paddingLeft: 1,
    })
    block.add(
      new TextRenderable(renderer, {
        content: file.title,
        fg: file.kind === "delete" ? theme.error : file.kind === "add" ? theme.success : theme.text,
        attributes: TextAttributes.BOLD,
        wrapMode: "word",
      }),
    )
    if (file.diff) {
      const diff = new DiffRenderable(renderer, {
        diff: file.diff,
        view: "split",
        filetype: guessPathFenceLanguage(file.filePath),
        syntaxStyle: transcriptSyntax,
        showLineNumbers: true,
        wrapMode: "none",
        fg: theme.text,
        addedBg: RGBA.fromHex("#102616"),
        removedBg: RGBA.fromHex("#2a1215"),
        contextBg: palette.element,
        addedSignColor: theme.success,
        removedSignColor: theme.error,
        lineNumberFg: theme.textMuted,
        lineNumberBg: palette.element,
        addedLineNumberBg: RGBA.fromHex("#16301d"),
        removedLineNumberBg: RGBA.fromHex("#34191d"),
      })
      block.add(diff)
      startTimeout(() => {
        ;(diff as unknown as { requestRebuild?: () => void }).requestRebuild?.()
        diff.requestRender()
      }, 0)
    } else {
      block.add(
        new TextRenderable(renderer, {
          content: file.kind === "delete" ? "File deleted" : "No diff available",
          fg: theme.textMuted,
          wrapMode: "word",
        }),
      )
    }
    body.add(block)
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
    appendAttachmentChip(text, tokenMime(match[1] ?? "file"), `[${(match[1] ?? "file").toLowerCase()} ${match[2] ?? "1"}]`)
    offset = index + match[0].length
  }
  if (offset < line.length) {
    appendTranscriptSpans(text, entry, line.slice(offset))
  }
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

function applyTranscriptTextContent(text: TextRenderable, entry: TranscriptEntry) {
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

function transcriptAccent(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return theme.primary
  }
  if (entry.role === "reasoning") {
    return theme.accent
  }
  if (entry.role === "tool") {
    return theme.secondary
  }
  if (entry.role === "error") {
    return theme.error
  }
  if (entry.role === "status") {
    return theme.info
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? theme.error
      : entry.emphasis === "warning"
        ? theme.warning
        : theme.textMuted
  }
  if (entry.role === "turn_summary") {
    return theme.borderSubtle
  }
  if (entry.role === "turn_toggle") {
    return theme.info
  }
  return theme.borderSubtle
}

function transcriptUsesAccentBorder(entry: TranscriptEntry) {
  return entry.role !== "status"
}

function transcriptBodyColor(entry: TranscriptEntry, surfaceTone: TranscriptSurfaceTone = "default") {
  const palette = transcriptSurfacePalette(surfaceTone)
  if (entry.role === "status") {
    return null
  }
  if (entry.role === "error") {
    return palette.panel
  }
  if (entry.role === "turn_summary") {
    return palette.panel
  }
  return entry.role === "assistant" || entry.role === "reasoning"
    ? palette.panel
    : palette.element
}

function transcriptTextColor(entry: TranscriptEntry) {
  if (entry.role === "user") {
    return theme.text
  }
  if (entry.role === "reasoning") {
    return theme.textMuted
  }
  if (entry.role === "tool") {
    return theme.secondary
  }
  if (entry.role === "error") {
    return theme.error
  }
  if (entry.role === "status") {
    return theme.info
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? theme.error
      : entry.emphasis === "warning"
        ? theme.warning
        : theme.textMuted
  }
  if (entry.role === "turn_summary") {
    return theme.text
  }
  if (entry.role === "turn_toggle") {
    return theme.info
  }
  return theme.text
}

function transcriptInlineCodeColor(entry: TranscriptEntry) {
  if (entry.role === "tool" || entry.role === "status" || entry.role === "error" || entry.role === "turn_toggle") {
    return theme.primary
  }
  if (entry.role === "user") {
    return theme.text
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error" ? theme.warning : theme.info
  }
  return theme.info
}
