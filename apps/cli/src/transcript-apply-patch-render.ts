import {
  BoxRenderable,
  RGBA,
  TextAttributes,
  TextNodeRenderable,
  TextRenderable,
} from "@opentui/core"

import { theme, TranscriptSeparatorBorder } from "./theme.js"
import { buildApplyPatchNewPreview } from "./transcript.js"
import type { TranscriptApplyPatchFiles } from "./transcript-render-mode.js"
import {
  transcriptSurfacePalette,
  type TranscriptSurfaceTone,
} from "./transcript-render-theme.js"

type RenderContext = ConstructorParameters<typeof BoxRenderable>[0]

export function buildApplyPatchTranscriptContent(
  renderer: RenderContext,
  body: BoxRenderable,
  files: TranscriptApplyPatchFiles,
  surfaceTone: TranscriptSurfaceTone,
) {
  const palette = transcriptSurfacePalette(surfaceTone)
  body.flexDirection = "column"
  body.gap = 0
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
      width: "100%",
      flexDirection: "column",
      border: ["bottom"],
      customBorderChars: TranscriptSeparatorBorder.customBorderChars,
      borderColor: theme.borderSubtle,
      paddingLeft: 0,
    })
    block.add(
      new TextRenderable(renderer, {
        content: file.title,
        fg: file.kind === "delete" ? theme.error : file.kind === "add" ? theme.success : theme.text,
        attributes: TextAttributes.BOLD,
        wrapMode: "word",
      }),
    )
    buildApplyPatchNewOnlyContent(renderer, block, file, palette.element)
    body.add(block)
  }
}

function buildApplyPatchNewOnlyContent(
  renderer: RenderContext,
  block: BoxRenderable,
  file: TranscriptApplyPatchFiles[number],
  contextBg: RGBA,
) {
  const preview = buildApplyPatchNewPreview(file)
  const text = new TextRenderable(renderer, {
    fg: theme.text,
    wrapMode: "none",
    bg: contextBg,
  })
  for (const [index, line] of preview.entries()) {
    if (index > 0) {
      text.add("\n")
    }
    if (line.kind === "added") {
      text.add(TextNodeRenderable.fromString(`+ ${line.text}`, {
        fg: theme.success,
        bg: RGBA.fromHex("#102616"),
      }))
      continue
    }
    if (line.kind === "meta") {
      text.add(TextNodeRenderable.fromString(line.text, {
        fg: theme.textMuted,
        attributes: TextAttributes.BOLD,
      }))
      continue
    }
    text.add(TextNodeRenderable.fromString(`  ${line.text}`, {
      fg: theme.text,
      bg: contextBg,
    }))
  }
  block.add(text)
}
