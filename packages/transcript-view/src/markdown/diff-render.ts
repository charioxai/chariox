import { diffStat, diffView, parseUnifiedDiff, type DiffFileView, type DiffRow } from "./diff-view.js"

// MP-08 / MP-10 (PANE-09, R16): Codex-style diff HTML for ```diff fences and
// tool patches: "Edited path (+N −M)", per-file rows for multi-file changes,
// one line-number gutter, tinted added/removed lines and ⋮ between hunks.

export type PatchFileInput = {
  readonly kind: DiffFileView["change"]
  readonly filePath: string
  readonly title?: string
  readonly diff: string | null
}

export function renderUnifiedDiffHtml(text: string): string {
  return renderDiffHtml(parseUnifiedDiff(text))
}

export function renderPatchFilesDiffHtml(files: readonly PatchFileInput[]): string {
  return renderDiffHtml(files.map((file) => {
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
  }))
}

export function renderDiffHtml(files: readonly DiffFileView[]): string {
  const view = diffView(files)
  const header = `<div class="freeform-diff-header"><span class="freeform-diff-verb">${escapeHtml(view.verb)}</span>${view.target ? ` <span class="freeform-diff-target">${escapeHtml(view.target)}</span>` : ""} ${stat(view.additions, view.deletions)}</div>`
  const body = files.map((file) => {
    const fileRow = files.length > 1
      ? `<div class="freeform-diff-file"><span class="freeform-diff-path">${escapeHtml(file.path ?? "diff")}</span> ${stat(file.additions, file.deletions)}</div>`
      : ""
    const rows = file.rows.length > 0
      ? file.rows.slice(0, 400).map(renderRow).join("")
      : `<div class="freeform-diff-meta">${file.change === "delete" ? "file deleted" : "no textual changes"}</div>`
    return `${fileRow}<div class="freeform-diff-lines" tabindex="0">${rows}</div>${renderRemainingRows(file.rows.slice(400))}`
  }).join("")
  return `<div class="freeform-diff" data-diff-additions="${view.additions}" data-diff-deletions="${view.deletions}">${header}${body}</div>`
}

function renderRow(row: DiffRow): string {
  if (row.kind === "gap") return `<div class="freeform-diff-gap" aria-label="unchanged lines">⋮</div>`
  if (row.kind === "meta") return `<div class="freeform-diff-meta">${escapeHtml(row.text)}</div>`
  const sign = row.line === "added" ? "+" : row.line === "removed" ? "-" : ""
  return `<div class="freeform-diff-row" data-kind="${row.line}"><span class="freeform-diff-ln">${row.number ?? ""}</span><span class="freeform-diff-sign">${sign}</span><span class="freeform-diff-code">${escapeHtml(row.text)}</span></div>`
}

function stat(additions: number, deletions: number): string {
  const [plus, minus] = diffStat(additions, deletions).slice(1, -1).split(" ")
  return `<span class="freeform-diff-stat">(<span class="freeform-diff-add">${plus}</span> <span class="freeform-diff-del">${minus}</span>)</span>`
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (match) => `&#${match.charCodeAt(0)};`)
}

function renderRemainingRows(rows: readonly DiffRow[]): string {
  if (rows.length === 0) return ""
  const text = rows.map(row => row.kind === "gap" ? "⋮" : row.kind === "meta" ? row.text : `${row.number ?? ""} ${row.line === "added" ? "+" : row.line === "removed" ? "-" : " "} ${row.text}`).join("\n")
  return `<details class="freeform-diff-overflow"><summary>Show ${rows.length} remaining lines</summary><pre tabindex="0">${escapeHtml(text)}</pre></details>`
}
