import {
  BoxRenderable,
  RGBA,
  TextAttributes,
  TextNodeRenderable,
  TextRenderable,
} from "@opentui/core"
import {
  diffStat,
  diffView,
  parseUnifiedDiff,
  type DiffFileView,
  type DiffRow,
} from "@chariox/tool-display"

import { theme } from "./theme.js"
import type { TranscriptApplyPatchFiles } from "./transcript-render-mode.js"

// MP-08 / MP-10 (PANE-09, R16): Codex-style diffs in the terminal for tool
// patches and ```diff fences: "• Edited path (+N −M)", per-file rows when
// several files changed, one line-number gutter, tinted added/removed lines
// and ⋮ between hunks.

type RenderContext = ConstructorParameters<typeof BoxRenderable>[0]

const addedBackground = RGBA.fromHex("#102616")
const removedBackground = RGBA.fromHex("#2b1215")

export class TranscriptDiffRenderable extends BoxRenderable {
  constructor(ctx: RenderContext, files: readonly DiffFileView[] = []) {
    super(ctx, { width: "100%", flexDirection: "column", gap: 0 })
    this.setFiles(files)
  }

  setFiles(files: readonly DiffFileView[]): void {
    for (const child of this.getChildren()) {
      this.remove(child.id)
      child.destroyRecursively()
    }
    const view = diffView(files)
    const header = new TextRenderable(this.ctx, { wrapMode: "word" })
    header.add(TextNodeRenderable.fromString("• ", { fg: theme.textMuted }))
    header.add(TextNodeRenderable.fromString(view.verb, { fg: theme.text, attributes: TextAttributes.BOLD }))
    if (view.target) header.add(TextNodeRenderable.fromString(` ${view.target}`, { fg: theme.text }))
    appendStat(header, view.additions, view.deletions)
    this.add(header)
    const width = gutterWidth(files)
    for (const file of files) {
      if (files.length > 1) {
        const row = new TextRenderable(this.ctx, { wrapMode: "word" })
        row.add(TextNodeRenderable.fromString(`  └ ${file.path ?? "diff"}`, { fg: theme.text }))
        appendStat(row, file.additions, file.deletions)
        this.add(row)
      }
      const lines = new TextRenderable(this.ctx, { wrapMode: "none" })
      for (const [index, row] of file.rows.entries()) {
        if (index > 0) lines.add("\n")
        appendRow(lines, row, width)
      }
      if (file.rows.length === 0) lines.add(TextNodeRenderable.fromString(file.change === "delete" ? "    file deleted" : "    no textual changes", { fg: theme.textMuted }))
      this.add(lines)
    }
    this.requestRender()
  }
}

export function applyPatchDiffFiles(files: TranscriptApplyPatchFiles): DiffFileView[] {
  return files.map((file) => {
    const parsed = parseUnifiedDiff(file.diff ?? "")
    const rows: DiffRow[] = parsed.flatMap((part) => part.rows)
    const commonTitles = ["Patched", "Created", "Deleted", "Edited", "Wrote"].map(verb => `${verb} ${file.filePath}`)
    if (file.title && !commonTitles.includes(file.title)) rows.unshift({ kind: "meta", text: file.title })
    return {
      path: file.filePath,
      previousPath: parsed[0]?.previousPath ?? null,
      change: file.kind,
      additions: parsed.reduce((sum, part) => sum + part.additions, 0),
      deletions: parsed.reduce((sum, part) => sum + part.deletions, 0),
      rows,
    }
  })
}

export function buildApplyPatchDiffContent(renderer: RenderContext, body: BoxRenderable, files: TranscriptApplyPatchFiles): void {
  body.flexDirection = "column"
  body.gap = 0
  body.add(new TranscriptDiffRenderable(renderer, applyPatchDiffFiles(files)))
}

export function isDiffFiletype(value: string | undefined): boolean {
  return /^(?:diff|patch)$/i.test(value ?? "")
}

function appendStat(text: TextRenderable, additions: number, deletions: number): void {
  const stat = diffStat(additions, deletions)
  const [plus, minus] = stat.slice(1, -1).split(" ")
  text.add(TextNodeRenderable.fromString(" (", { fg: theme.textMuted }))
  text.add(TextNodeRenderable.fromString(plus!, { fg: theme.success }))
  text.add(TextNodeRenderable.fromString(" ", { fg: theme.textMuted }))
  text.add(TextNodeRenderable.fromString(minus!, { fg: theme.error }))
  text.add(TextNodeRenderable.fromString(")", { fg: theme.textMuted }))
}

function appendRow(text: TextRenderable, row: DiffRow, width: number): void {
  if (row.kind === "gap") {
    text.add(TextNodeRenderable.fromString(`${" ".repeat(width)} ⋮`, { fg: theme.textMuted }))
    return
  }
  if (row.kind === "meta") {
    text.add(TextNodeRenderable.fromString(`${" ".repeat(width)}   ${row.text}`, { fg: theme.textMuted, attributes: TextAttributes.ITALIC }))
    return
  }
  const number = row.number === null ? "" : String(row.number)
  text.add(TextNodeRenderable.fromString(`${number.padStart(width)} `, { fg: theme.textMuted }))
  if (row.line === "added") {
    text.add(TextNodeRenderable.fromString(`+ ${row.text}`, { fg: theme.success, bg: addedBackground }))
  } else if (row.line === "removed") {
    text.add(TextNodeRenderable.fromString(`- ${row.text}`, { fg: theme.error, bg: removedBackground }))
  } else {
    text.add(TextNodeRenderable.fromString(`  ${row.text}`, { fg: theme.text }))
  }
}

function gutterWidth(files: readonly DiffFileView[]): number {
  const largest = Math.max(0, ...files.flatMap((file) => file.rows.map((row) => row.kind === "line" ? row.number ?? 0 : 0)))
  return largest > 0 ? String(largest).length : 0
}
