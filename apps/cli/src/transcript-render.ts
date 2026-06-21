import {
  BoxRenderable,
  MarkdownRenderable,
  MouseButton,
  TextAttributes,
  TextRenderable,
  type SyntaxStyle,
} from "@opentui/core"
import { setTimeout as startTimeout } from "node:timers"

import type { TranscriptEntry } from "./cli-types.js"
import { transcriptEntryPadding } from "./transcript-entry-style.js"
import { theme, TranscriptSeparatorBorder } from "./theme.js"
import {
  normalizeMarkdownFenceInfoStrings,
  shouldRenderTranscriptAsMarkdown,
} from "./transcript.js"
import { buildApplyPatchTranscriptContent } from "./transcript-apply-patch-render.js"
import {
  buildBlobToggleLabel,
  buildCollapsedTranscriptBlob,
  buildTurnToggleContent,
} from "./transcript-interaction-render.js"
import {
  readTranscriptApplyPatch,
  shouldRenderCollapsedTranscriptBlob,
  transcriptRenderMode,
} from "./transcript-render-mode.js"
import {
  transcriptAccent,
  transcriptBodyColor,
  transcriptTextColor,
  transcriptUsesSeparator,
  type TranscriptSurfaceTone,
} from "./transcript-render-theme.js"
import { applyTranscriptTextContent } from "./transcript-text-render.js"

export {
  transcriptRenderMode,
} from "./transcript-render-mode.js"

export {
  resolveTranscriptSurfaceTone,
  transcriptSurfacePalette,
  type TranscriptSurfaceTone,
} from "./transcript-render-theme.js"

type RenderContext = ConstructorParameters<typeof BoxRenderable>[0]

export type TranscriptEntryRenderable = {
  entry: TranscriptEntry
  wrapper: BoxRenderable
  update: (entry: TranscriptEntry) => void
}

export type QueuedPromptAction = "steer" | "cancel"

export function buildTranscriptEntryRenderable(
  renderer: RenderContext,
  entry: TranscriptEntry,
  transcriptSyntax: SyntaxStyle,
  onToggleTurn: (turnId: number | null | undefined, toggleEntryId?: number) => void,
  onToggleBlob: (entryId: number, collapsed: boolean) => void,
  surfaceTone: TranscriptSurfaceTone = "default",
  onQueuedPromptAction?: (entry: TranscriptEntry, action: QueuedPromptAction) => void,
) {
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 0,
    width: "100%",
    flexDirection: "column",
  })
  let currentEntry = entry
  let currentMode = transcriptRenderMode(entry)
  let body: BoxRenderable | null = null
  let textRenderable: TextRenderable | null = null
  let markdownRenderable: MarkdownRenderable | null = null
  let update: (nextEntry: TranscriptEntry) => void

  const render = (nextEntry: TranscriptEntry) => {
    for (const child of wrapper.getChildren()) {
      wrapper.remove(child.id)
      child.destroyRecursively()
    }
    body = null
    textRenderable = null
    markdownRenderable = null

    const bodyColor = transcriptBodyColor(nextEntry, surfaceTone)
    const padding = transcriptEntryPadding(nextEntry)
    body = new BoxRenderable(renderer, {
      width: "100%",
      paddingLeft: padding.horizontal,
      paddingRight: padding.horizontal,
      paddingTop: padding.vertical,
      paddingBottom: padding.vertical,
      flexDirection: "column",
      ...(bodyColor ? { backgroundColor: bodyColor } : {}),
    })
    if (transcriptUsesSeparator(nextEntry)) {
      body.border = ["bottom"]
      body.customBorderChars = TranscriptSeparatorBorder.customBorderChars
      body.borderColor = transcriptAccent(nextEntry)
    }

    if (shouldRenderCollapsedTranscriptBlob(nextEntry)) {
      buildCollapsedTranscriptBlob(renderer, body, nextEntry, onToggleBlob)
    } else if (nextEntry.role === "turn_toggle") {
      buildTurnToggleContent(renderer, body, nextEntry, onToggleTurn)
    } else {
      const expanded = buildExpandedTranscriptContent(
        renderer,
        body,
        nextEntry,
        transcriptSyntax,
        surfaceTone,
        onQueuedPromptAction,
      )
      textRenderable = expanded.textRenderable
      markdownRenderable = expanded.markdownRenderable
      if (nextEntry.blobCollapsible) {
        body.add(buildBlobToggleLabel(renderer, "click to collapse", () => onToggleBlob(nextEntry.id, true)))
      }
    }

    wrapper.add(body)
  }

  const fastUpdate = (nextEntry: TranscriptEntry) => {
    if (currentMode === "text" && textRenderable) {
      applyTranscriptTextContent(textRenderable, nextEntry)
      textRenderable.fg = transcriptTextColor(nextEntry)
      textRenderable.requestRender()
      wrapper.requestRender()
      return true
    }

    if (currentMode === "markdown" && markdownRenderable) {
      markdownRenderable.content = normalizeMarkdownFenceInfoStrings(nextEntry.text)
      markdownRenderable.requestRender()
      wrapper.requestRender()
      return true
    }

    return false
  }

  render(entry)
  update = (nextEntry: TranscriptEntry) => {
    const nextMode = transcriptRenderMode(nextEntry)
    const canFastUpdate =
      nextMode === currentMode
      && nextEntry.role === currentEntry.role
      && nextEntry.emphasis === currentEntry.emphasis
      && nextEntry.blobCollapsible === currentEntry.blobCollapsible
      && nextEntry.blobCollapsed === currentEntry.blobCollapsed
      && nextEntry.blobTitle === currentEntry.blobTitle
      && nextEntry.blobSummary === currentEntry.blobSummary
      && nextEntry.queuedPrompt?.status === currentEntry.queuedPrompt?.status
      && nextEntry.queuedPrompt?.promptId === currentEntry.queuedPrompt?.promptId
      && nextEntry.queuedPrompt?.agentId === currentEntry.queuedPrompt?.agentId
    currentEntry = nextEntry
    if (canFastUpdate && fastUpdate(nextEntry)) {
      return
    }
    currentMode = nextMode
    render(nextEntry)
  }

  return { entry, wrapper, update }
}

export function renderPromptTranscript(prompt: string) {
  const text = prompt.trimEnd()
  return text ? `${text}\n` : ""
}

function buildExpandedTranscriptContent(
  renderer: RenderContext,
  body: BoxRenderable,
  entry: TranscriptEntry,
  transcriptSyntax: SyntaxStyle,
  surfaceTone: TranscriptSurfaceTone,
  onQueuedPromptAction?: (entry: TranscriptEntry, action: QueuedPromptAction) => void,
) {
  if (entry.queuedPrompt) {
    buildQueuedPromptTranscriptContent(renderer, body, entry, onQueuedPromptAction)
    return { textRenderable: null, markdownRenderable: null }
  }

  const patch = readTranscriptApplyPatch(entry)
  if (patch) {
    buildApplyPatchTranscriptContent(renderer, body, patch, surfaceTone)
    return { textRenderable: null, markdownRenderable: null }
  }

  if (shouldRenderTranscriptAsMarkdown(entry.role, entry.text)) {
    const markdown = new MarkdownRenderable(renderer, {
      content: normalizeMarkdownFenceInfoStrings(entry.text),
      syntaxStyle: transcriptSyntax,
      conceal: true,
      concealCode: false,
      streaming: true,
    })
    body.add(markdown)
    markdown.requestRender()
    return { textRenderable: null, markdownRenderable: markdown }
  }

  const text = new TextRenderable(renderer, {
    fg: transcriptTextColor(entry),
    wrapMode: "word",
  })
  applyTranscriptTextContent(text, entry)
  body.add(text)
  return { textRenderable: text, markdownRenderable: null }
}

function buildQueuedPromptTranscriptContent(
  renderer: RenderContext,
  body: BoxRenderable,
  entry: TranscriptEntry,
  onQueuedPromptAction?: (entry: TranscriptEntry, action: QueuedPromptAction) => void,
) {
  const row = new BoxRenderable(renderer, {
    width: "100%",
    flexDirection: "row",
    justifyContent: "space-between",
  })
  const message = new TextRenderable(renderer, {
    content: entry.text,
    fg: transcriptTextColor(entry),
    wrapMode: "word",
  })
  message.flexGrow = 1
  row.add(message)

  const actions = new BoxRenderable(renderer, {
    flexDirection: "row",
    flexGrow: 0,
  })
  actions.add(new TextRenderable(renderer, {
    content: " queued ",
    fg: theme.textMuted,
    attributes: TextAttributes.BOLD,
  }))
  actions.add(buildQueuedPromptActionLabel(renderer, "steer", entry, "steer", onQueuedPromptAction))
  actions.add(new TextRenderable(renderer, { content: " ", fg: theme.textMuted }))
  actions.add(buildQueuedPromptActionLabel(renderer, "cancel", entry, "cancel", onQueuedPromptAction))
  row.add(actions)
  body.add(row)
}

function buildQueuedPromptActionLabel(
  renderer: RenderContext,
  label: string,
  entry: TranscriptEntry,
  action: QueuedPromptAction,
  onQueuedPromptAction?: (entry: TranscriptEntry, action: QueuedPromptAction) => void,
) {
  const disabled = entry.queuedPrompt?.status === "steering" || entry.queuedPrompt?.status === "cancelling"
  const text = new TextRenderable(renderer, {
    content: disabled ? label : `[${label}]`,
    fg: disabled ? theme.textMuted : theme.primary,
    attributes: disabled ? TextAttributes.NONE : TextAttributes.BOLD,
  })
  text.onMouseUp = (event) => {
    if (disabled || event.button !== MouseButton.LEFT) {
      return
    }
    event.stopPropagation()
    startTimeout(() => {
      onQueuedPromptAction?.(entry, action)
    }, 0)
  }
  return text
}
