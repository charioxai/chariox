import { isObjectValue, readString } from "./strings.js"
import type { ApplyPatchFile, ToolTranscriptUpdate } from "./types.js"

// MP-08 / MP-10 (PANE-09, R16): string-replacement edit tools carry the
// before/after text, not a patch. Turn OpenCode `edit`/`write` and Claude
// `Edit`/`MultiEdit`/`Write` inputs into a unified diff so they render with
// the same diff view as Codex apply_patch. Line numbers are unknown, so hunks
// have no ranges; unchanged leading/trailing lines are context.

type Replacement = { readonly before: string; readonly after: string }

export function readEditToolFiles(update: ToolTranscriptUpdate): ApplyPatchFile[] {
  const tool = (update.tool ?? "").toLowerCase()
  if (tool !== "edit" && tool !== "multiedit" && tool !== "write") return []
  // Inputs describe intent until the official harness reports completion.
  if (update.status !== "completed" || update.error?.trim()) return []
  const input = isObjectValue(update.input) ? update.input as Record<string, unknown> : null
  const filePath = input ? readString(input.filePath) ?? readString(input.file_path) ?? readString(input.path) : null
  if (!input || !filePath) return []
  if (tool === "write") {
    const content = stringField(input.content)
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

function readReplacement(value: unknown): Replacement[] {
  if (!isObjectValue(value)) return []
  const record = value as Record<string, unknown>
  const before = stringField(record.oldString) ?? stringField(record.old_string)
  const after = stringField(record.newString) ?? stringField(record.new_string)
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

function stringField(value: unknown): string | null {
  return typeof value === "string" ? value : null
}

function splitLines(value: string): string[] {
  if (value === "") return []
  const lines = value.replace(/\r\n/g, "\n").split("\n")
  if (lines.length > 1 && lines.at(-1) === "") lines.pop()
  return lines
}
