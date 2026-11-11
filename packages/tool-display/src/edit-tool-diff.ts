import { compactToolName } from "./tool-names.js"
import { isObjectValue, readString } from "./strings.js"
import type { ApplyPatchFile, ToolTranscriptUpdate } from "./types.js"

// MP-08 / MP-10 (PANE-09, R16): string-replacement edit tools carry the
// before/after text, not a patch. Turn OpenCode `edit`/`write` and Claude
// `Edit`/`MultiEdit`/`Write` inputs into a unified diff so they render with
// the same diff view as Codex apply_patch. Line numbers are unknown, so hunks
// have no ranges; unchanged leading/trailing lines are context.

type Replacement = { readonly before: string; readonly after: string }

// The Chariox MCP bridge names its tools "chariox.write_artifact", "mcp__chariox__chariox_write_artifact" and the like.
function editToolName(tool: string | undefined): string {
  const compact = compactToolName(tool)
  if (compact.endsWith("writeartifact")) return "write"
  if (compact.endsWith("editartifact")) return "edit"
  if (compact.endsWith("deleteartifact")) return "delete"
  if (compact.endsWith("moveartifact")) return "move"
  return compact === "writefile" || compact === "create" ? "write" : compact
}

// A call that failed, was refused or stopped changed nothing: what it shows is its error.
const UNCHANGED = /^(?:error|failed|declined|denied|rejected|cancel+ed)$/i

/** The files a write, an edit, a delete or a move names, with what changed. Shown while the call runs as well as once it is done. */
export function readEditToolFiles(update: ToolTranscriptUpdate): ApplyPatchFile[] {
  const tool = editToolName(update.tool)
  if (tool !== "edit" && tool !== "multiedit" && tool !== "write" && tool !== "delete" && tool !== "move") return []
  if (update.error?.trim() || UNCHANGED.test(update.status ?? "")) return []
  // An edit's diff is drawn once the harness reports it done; a write's content, a delete and a move are known from the start.
  if ((tool === "edit" || tool === "multiedit") && update.status !== "completed") return []
  const input = isObjectValue(update.input) ? update.input as Record<string, unknown> : null
  if (!input) return []
  if (tool === "move") {
    const from = readString(input.from_path) ?? readString(input.fromPath)
    const to = readString(input.to_path) ?? readString(input.toPath)
    return from ? [{ kind: "move", filePath: from, title: to ? `Moved ${from} -> ${to}` : `Moved ${from}`, diff: null, ...to ? { note: `Moved to ${to}` } : {} }] : []
  }
  const filePath = readString(input.filePath) ?? readString(input.file_path) ?? readString(input.path)
  if (!filePath) return []
  if (tool === "delete") return [{ kind: "delete", filePath, title: `Deleted ${filePath}`, diff: null }]
  // Chariox's edit_artifact without `old_text` replaces the whole file with `new_text`.
  const wholeFile = tool === "write" || (tool === "edit" && stringField(input.old_text) === null && stringField(input.oldString) === null && stringField(input.old_string) === null && stringField(input.new_text) !== null)
  if (wholeFile) {
    // Chariox's write_artifact holds text in `content_text` and a binary file in `content_base64`.
    const binary = stringField(input.content_base64)
    if (binary !== null) {
      return [{ kind: "update", filePath, title: `Wrote ${filePath}`, diff: null, note: `Binary file · ${formatBytes(Math.floor(binary.length * 3 / 4))}` }]
    }
    const content = stringField(input.content) ?? stringField(input.content_text) ?? stringField(input.new_text)
    if (content === null) return []
    const lines = splitLines(content)
    return [{
      // Write may overwrite an existing file; the input has no old snapshot.
      kind: "update",
      filePath,
      title: `Wrote ${filePath}`,
      diff: [`--- a/${filePath}`, `+++ b/${filePath}`, "@@", ...lines.map((line) => `+${line}`)].join("\n"),
    }]
  }
  const replacements = tool === "multiedit"
    ? (Array.isArray(input.edits) ? input.edits : []).flatMap(readReplacement)
    : readReplacement(input)
  if (replacements.length === 0) return []
  return [{
    kind: "update",
    filePath,
    title: `Edited ${filePath}`,
    diff: [`--- a/${filePath}`, `+++ b/${filePath}`, ...replacements.flatMap(replacementHunk)].join("\n"),
  }]
}

/** The file a write, edit, delete or move names, when nothing is known of its change: the row still opens to its path. */
export function readNamedEditFile(update: ToolTranscriptUpdate): ApplyPatchFile | null {
  const tool = editToolName(update.tool)
  if (tool !== "edit" && tool !== "multiedit" && tool !== "write" && tool !== "delete" && tool !== "move") return null
  const input = isObjectValue(update.input) ? update.input as Record<string, unknown> : null
  const filePath = input ? readString(input.filePath) ?? readString(input.file_path) ?? readString(input.path) ?? readString(input.from_path) : null
  if (!filePath) return null
  const verb = tool === "write" ? "Wrote" : tool === "delete" ? "Deleted" : tool === "move" ? "Moved" : "Edited"
  return { kind: tool === "delete" ? "delete" : tool === "move" ? "move" : "update", filePath, title: `${verb} ${filePath}`, diff: null }
}

function readReplacement(value: unknown): Replacement[] {
  if (!isObjectValue(value)) return []
  const record = value as Record<string, unknown>
  const before = stringField(record.oldString) ?? stringField(record.old_string) ?? stringField(record.old_text)
  const after = stringField(record.newString) ?? stringField(record.new_string) ?? stringField(record.new_text)
  return before === null || after === null ? [] : [{ before, after }]
}

function replacementHunk({ before, after }: Replacement): string[] {
  const oldLines = splitLines(before)
  const newLines = splitLines(after)
  let prefix = 0
  while (prefix < oldLines.length && prefix < newLines.length && oldLines[prefix] === newLines[prefix]) prefix += 1
  let suffix = 0
  while (
    suffix < oldLines.length - prefix && suffix < newLines.length - prefix
    && oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) suffix += 1
  return [
    "@@",
    ...oldLines.slice(0, prefix).map((line) => ` ${line}`),
    ...oldLines.slice(prefix, oldLines.length - suffix).map((line) => `-${line}`),
    ...newLines.slice(prefix, newLines.length - suffix).map((line) => `+${line}`),
    ...oldLines.slice(oldLines.length - suffix).map((line) => ` ${line}`),
  ]
}

function formatBytes(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

function stringField(value: unknown): string | null {
  return typeof value === "string" ? value : null
}

function splitLines(value: string): string[] {
  if (value === "") return []
  const lines = value.replace(/\r\n/g, "\n").split("\n")
  if (lines.length > 1 && lines.at(-1) === "") lines.pop()
  return lines
}
