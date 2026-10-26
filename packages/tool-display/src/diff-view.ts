// MP-08 / MP-10 (PANE-09, ROADMAP R16): one diff view model for tool patches
// and ```diff fences, rendered Codex-style by every client: header verb +
// (+N −M), per-file rows, one line-number gutter (new number for added and
// context lines, old number for removed lines), and a gap marker between
// hunks. Mirrored byte-for-byte in Cloud apps/web/src/markdown/diff-view.ts.

export type DiffLineKind = "added" | "removed" | "context"

export type DiffRow =
  | { readonly kind: "line"; readonly line: DiffLineKind; readonly number: number | null; readonly text: string }
  | { readonly kind: "gap" }
  | { readonly kind: "meta"; readonly text: string }

export type DiffFileChange = "add" | "delete" | "update" | "move"

export type DiffFileView = {
  readonly path: string | null
  readonly previousPath: string | null
  readonly change: DiffFileChange
  readonly additions: number
  readonly deletions: number
  readonly rows: readonly DiffRow[]
}

export type DiffView = {
  readonly verb: string
  readonly target: string
  readonly additions: number
  readonly deletions: number
  readonly files: readonly DiffFileView[]
}

const hunkPattern = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/

export function parseUnifiedDiff(text: string): DiffFileView[] {
  const files: MutableFile[] = []
  let file: MutableFile | null = null
  let oldLine: number | null = null
  let newLine: number | null = null
  let sawHunk = false
  const current = () => {
    file ??= newFile()
    if (!files.includes(file)) files.push(file)
    return file
  }
  const lines = text.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n")
  for (const [index, raw] of lines.entries()) {
    if (raw.startsWith("diff --git ")) {
      file = newFile()
      files.push(file)
      const paths = /^diff --git a\/(.+?) b\/(.+)$/.exec(raw)
      if (paths) [file.previousPath, file.path] = [paths[1]!, paths[2]!]
      sawHunk = false
      continue
    }
    // Inside a hunk "--- x" is a removed "-- x" unless a "+++"/"@@" header follows.
    if (raw.startsWith("--- ") && (!sawHunk || (lines[index + 1]?.startsWith("+++ ") && lines[index + 2]?.startsWith("@@")))) {
      if (file && (file.rows.length > 0 || file.pathFromHeaders)) {
        file = newFile()
        files.push(file)
      }
      const target = current()
      const path = headerPath(raw.slice(4))
      target.previousPath = path
      if (path === null) target.change = "add"
      target.pathFromHeaders = true
      sawHunk = false
      continue
    }
    if (raw.startsWith("+++ ") && file && file.pathFromHeaders && !sawHunk) {
      const path = headerPath(raw.slice(4))
      file.path = path ?? file.previousPath
      if (path === null) file.change = "delete"
      continue
    }
    if (/^(?:index |similarity index |old mode |new mode )/.test(raw)) continue
    if (raw.startsWith("new file mode ")) { current().change = "add"; continue }
    if (raw.startsWith("deleted file mode ")) { current().change = "delete"; continue }
    if (raw.startsWith("rename from ")) { current().previousPath = raw.slice(12); current().change = "move"; continue }
    if (raw.startsWith("rename to ")) { current().path = raw.slice(10); current().change = "move"; continue }
    if (raw.startsWith("@@")) {
      const target = current()
      const numbers = hunkPattern.exec(raw)
      oldLine = numbers ? Number(numbers[1]) : null
      newLine = numbers ? Number(numbers[2]) : null
      if (sawHunk || target.rows.length > 0) target.rows.push({ kind: "gap" })
      sawHunk = true
      continue
    }
    if (raw.startsWith("\\")) {
      current().rows.push({ kind: "meta", text: raw.slice(1).trim() })
      continue
    }
    const target = current()
    const sign = raw[0]
    if (sign === "+") {
      target.rows.push({ kind: "line", line: "added", number: newLine, text: raw.slice(1) })
      target.additions += 1
      newLine = newLine === null ? null : newLine + 1
    } else if (sign === "-") {
      target.rows.push({ kind: "line", line: "removed", number: oldLine, text: raw.slice(1) })
      target.deletions += 1
      oldLine = oldLine === null ? null : oldLine + 1
    } else {
      target.rows.push({ kind: "line", line: "context", number: newLine, text: sign === " " ? raw.slice(1) : raw })
      oldLine = oldLine === null ? null : oldLine + 1
      newLine = newLine === null ? null : newLine + 1
    }
  }
  return files.map(({ pathFromHeaders: _pathFromHeaders, ...view }) => view)
}

export function diffView(files: readonly DiffFileView[]): DiffView {
  const additions = files.reduce((sum, file) => sum + file.additions, 0)
  const deletions = files.reduce((sum, file) => sum + file.deletions, 0)
  const only = files.length === 1 ? files[0]! : null
  const changes = new Set(files.map((file) => file.change))
  const verb = changes.size === 1 ? changeVerb([...changes][0]!) : "Edited"
  const target = only
    ? only.change === "move" && only.previousPath && only.path ? `${only.previousPath} → ${only.path}` : only.path ?? ""
    : `${files.length} files`
  return { verb: only?.path || files.length > 1 ? verb : "Diff", target, additions, deletions, files }
}

export function diffStat(additions: number, deletions: number): string {
  return `(+${additions} −${deletions})`
}

function changeVerb(change: DiffFileChange): string {
  return change === "add" ? "Added" : change === "delete" ? "Deleted" : change === "move" ? "Moved" : "Edited"
}

type MutableFile = {
  path: string | null
  previousPath: string | null
  change: DiffFileChange
  additions: number
  deletions: number
  rows: DiffRow[]
  pathFromHeaders: boolean
}

function newFile(): MutableFile {
  return { path: null, previousPath: null, change: "update", additions: 0, deletions: 0, rows: [], pathFromHeaders: false }
}

function headerPath(value: string): string | null {
  const path = value.split("\t")[0]!.trim()
  if (path === "/dev/null") return null
  return path.replace(/^[ab]\//, "")
}
