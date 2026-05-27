import { formatToolStatusBadge } from "./status.js"
import {
  isObjectValue,
  nonEmpty,
  normalizeJsonLike,
  normalizeToolOutputPayload,
  readString,
} from "./strings.js"
import { isWorkspaceLiveSyncTool } from "./tool-names.js"
import type {
  ApplyPatchFile,
  ToolDisplayPatchLine,
  ToolTranscriptUpdate,
} from "./types.js"

type ApplyPatchInput = {
  patchText?: unknown
}

type CodexFileChange = {
  path?: unknown
  filePath?: unknown
  kind?: unknown
  type?: unknown
  diff?: unknown
  unified_diff?: unknown
  unifiedDiff?: unknown
  patch?: unknown
  move_path?: unknown
  movePath?: unknown
}

type WorkspaceLiveSyncChange = {
  path?: unknown
  kind?: unknown
  diff?: unknown
  diff_truncated?: unknown
  diffTruncated?: unknown
}

export function buildApplyPatchNewPreview(file: ApplyPatchFile): ToolDisplayPatchLine[] {
  if (!file.diff) {
    return [{ kind: "meta", text: file.kind === "delete" ? "File deleted" : "No diff available" }]
  }
  const lines: ToolDisplayPatchLine[] = []
  for (const line of file.diff.split(/\r?\n/)) {
    if (!line || line.startsWith("diff --git") || line.startsWith("index ") || line.startsWith("--- ") || line.startsWith("+++ ")) {
      continue
    }
    if (line.startsWith("@@")) {
      lines.push({ kind: "meta", text: line })
      continue
    }
    if (line.startsWith("+")) {
      lines.push({ kind: "added", text: line.slice(1) })
      continue
    }
    if (line.startsWith("-")) {
      continue
    }
    if (line.startsWith(" ")) {
      lines.push({ kind: "context", text: line.slice(1) })
      continue
    }
    if (line.startsWith("rename from ") || line.startsWith("rename to ") || line.startsWith("new file mode ") || line.startsWith("deleted file mode ")) {
      lines.push({ kind: "meta", text: line })
      continue
    }
    lines.push({ kind: "context", text: line })
  }
  return lines.length > 0 ? lines : [{ kind: "meta", text: "No visible new changes" }]
}

export function readApplyPatchFiles(update: ToolTranscriptUpdate) {
  const managedFiles = readWorkspaceLiveSyncChangeFiles(update)
  if (managedFiles.length > 0) {
    return managedFiles
  }

  if (update.tool !== "apply_patch") {
    return []
  }
  const patchFiles = parseApplyPatchText(readApplyPatchText(update.input))
  if (patchFiles.length > 0) {
    return patchFiles
  }

  for (const source of [update.input, update.raw, update.output]) {
    const files = readCodexFileChangeFiles(source)
    if (files.length > 0) {
      return files
    }
  }

  const streamedPatchFiles = parseApplyPatchText(
    [readApplyPatchText(update.raw), readApplyPatchText(update.output)].find((value) => value.trim()) ?? "",
  )
  return streamedPatchFiles
}

export function formatApplyPatchTranscriptUpdate(update: ToolTranscriptUpdate) {
  const files = readApplyPatchFiles(update)
  if (files.length === 0) {
    return null
  }

  const kinds = files.reduce(
    (acc: Record<ApplyPatchFile["kind"], number>, file: ApplyPatchFile) => {
      acc[file.kind] += 1
      return acc
    },
    { add: 0, delete: 0, move: 0, update: 0 },
  )
  const parts = [
    kinds.update ? `${kinds.update} updated` : "",
    kinds.add ? `${kinds.add} added` : "",
    kinds.move ? `${kinds.move} moved` : "",
    kinds.delete ? `${kinds.delete} deleted` : "",
  ].filter(Boolean)

  return [
    `**patch**${formatToolStatusBadge(nonEmpty(update.status))}`,
    `${files.length} ${files.length === 1 ? "file" : "files"}${parts.length ? ` · ${parts.join(", ")}` : ""}`,
    ...files.slice(0, 6).map((file: ApplyPatchFile) => `- ${file.title}`),
  ].join("\n")
}

function readApplyPatchText(input: unknown) {
  if (typeof input === "string") {
    return input.includes("*** Begin Patch") ? input : ""
  }
  if (!input || typeof input !== "object") {
    return ""
  }
  const patchText = (input as ApplyPatchInput & { patch_text?: unknown }).patchText
    ?? (input as { patch_text?: unknown }).patch_text
  return typeof patchText === "string" ? patchText : ""
}

function readCodexFileChangeFiles(value: unknown): ApplyPatchFile[] {
  const normalized = normalizeJsonLike(value)
  const changes = readCodexFileChangeList(normalized)
  if (!changes) {
    return []
  }

  return changes
    .map(readCodexFileChange)
    .filter((file): file is ApplyPatchFile => Boolean(file))
}

function readWorkspaceLiveSyncChangeFiles(update: ToolTranscriptUpdate): ApplyPatchFile[] {
  if (!isWorkspaceLiveSyncTool(update.tool)) {
    return []
  }

  for (const source of [update.output, update.raw]) {
    const normalized = normalizeToolOutputPayload(source)
    if (!isObjectValue(normalized)) {
      continue
    }
    const changes = normalized.changes
    if (Array.isArray(changes)) {
      const files = changes
        .map(readWorkspaceLiveSyncChange)
        .filter((file): file is ApplyPatchFile => Boolean(file))
      if (files.length > 0) {
        return files
      }
    }
    const file = readWorkspaceLiveSyncChange(normalized.change)
    if (file) {
      return [file]
    }
  }

  return []
}

function readWorkspaceLiveSyncChange(value: unknown): ApplyPatchFile | null {
  if (!isObjectValue(value)) {
    return null
  }
  const change = value as WorkspaceLiveSyncChange
  const filePath = readString(change.path)
  const diff = readString(change.diff)
  if (!filePath || !diff) {
    return null
  }
  const kind = normalizeFileChangeKind(readString(change.kind), null)
  const truncated = change.diff_truncated === true || change.diffTruncated === true
  return {
    kind,
    filePath,
    title: `${codexFileChangeTitle(kind, filePath, null)}${truncated ? " (diff truncated)" : ""}`,
    diff: buildCodexFileChangeDiff(filePath, null, kind, diff),
  }
}

function readCodexFileChangeList(value: unknown): CodexFileChange[] | null {
  if (Array.isArray(value)) {
    return value.filter(isObjectValue)
  }
  if (!isObjectValue(value)) {
    return null
  }
  const changes = value.changes
  if (Array.isArray(changes)) {
    return changes.filter(isObjectValue)
  }
  if (isObjectValue(changes)) {
    return Object.entries(changes).map(([path, change]) => (
      isObjectValue(change) ? { path, ...change } : { path }
    ))
  }
  return null
}

function readCodexFileChange(change: CodexFileChange): ApplyPatchFile | null {
  const filePath = readString(change.path) ?? readString(change.filePath)
  if (!filePath) {
    return null
  }
  const movePath = readString(change.move_path) ?? readString(change.movePath)
  const kind = normalizeFileChangeKind(readString(change.kind) ?? readString(change.type), movePath)
  const diffText =
    readString(change.diff)
    ?? readString(change.unified_diff)
    ?? readString(change.unifiedDiff)
    ?? readString(change.patch)
  return {
    kind,
    filePath: movePath ?? filePath,
    title: codexFileChangeTitle(kind, filePath, movePath),
    diff: kind === "delete" && !diffText
      ? null
      : buildCodexFileChangeDiff(filePath, movePath, kind, diffText ?? ""),
  }
}

function normalizeFileChangeKind(value: string | null, movePath: string | null): ApplyPatchFile["kind"] {
  if (movePath) {
    return "move"
  }
  switch (value?.toLowerCase()) {
    case "add":
    case "added":
    case "create":
    case "created":
      return "add"
    case "delete":
    case "deleted":
    case "remove":
    case "removed":
      return "delete"
    case "move":
    case "moved":
    case "rename":
    case "renamed":
      return "move"
    default:
      return "update"
  }
}

function codexFileChangeTitle(kind: ApplyPatchFile["kind"], filePath: string, movePath: string | null) {
  switch (kind) {
    case "add":
      return `Created ${filePath}`
    case "delete":
      return `Deleted ${filePath}`
    case "move":
      return movePath ? `Moved ${filePath} -> ${movePath}` : `Moved ${filePath}`
    case "update":
      return `Patched ${filePath}`
  }
}

function buildCodexFileChangeDiff(
  filePath: string,
  movePath: string | null,
  kind: ApplyPatchFile["kind"],
  diffText: string,
) {
  const trimmed = diffText.trimEnd()
  if (trimmed.includes("diff --git")) {
    return trimmed
  }

  const previous = normalizeDiffPath(filePath)
  const next = normalizeDiffPath(movePath ?? filePath)
  const header = [`diff --git a/${previous} b/${next}`]
  if (kind === "add") {
    header.push("new file mode 100644")
    header.push("--- /dev/null")
    header.push(`+++ b/${next}`)
  } else if (kind === "delete") {
    header.push(`--- a/${previous}`)
    header.push("+++ /dev/null")
  } else {
    if (movePath) {
      header.push(`rename from ${previous}`)
      header.push(`rename to ${next}`)
    }
    header.push(`--- a/${previous}`)
    header.push(`+++ b/${next}`)
  }
  return trimmed ? [...header, trimmed].join("\n") : header.join("\n")
}

function parseApplyPatchText(text: string) {
  if (!text.trim()) {
    return [] as ApplyPatchFile[]
  }

  const lines = text.split(/\r?\n/)
  const files: ApplyPatchFile[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i] ?? ""
    if (line.startsWith("*** Add File: ")) {
      const filePath = line.slice("*** Add File: ".length).trim()
      const body: string[] = []
      i += 1
      while (i < lines.length && !lines[i]!.startsWith("*** ")) {
        body.push(lines[i]!)
        i += 1
      }
      files.push({
        kind: "add",
        filePath,
        title: `Created ${filePath}`,
        diff: buildAddedDiff(filePath, body),
      })
      continue
    }
    if (line.startsWith("*** Delete File: ")) {
      const filePath = line.slice("*** Delete File: ".length).trim()
      files.push({
        kind: "delete",
        filePath,
        title: `Deleted ${filePath}`,
        diff: null,
      })
      i += 1
      continue
    }
    if (line.startsWith("*** Update File: ")) {
      const filePath = line.slice("*** Update File: ".length).trim()
      let movePath: string | null = null
      const body: string[] = []
      i += 1
      if ((lines[i] ?? "").startsWith("*** Move to: ")) {
        movePath = lines[i]!.slice("*** Move to: ".length).trim()
        i += 1
      }
      while (i < lines.length && !lines[i]!.startsWith("*** ")) {
        body.push(lines[i]!)
        i += 1
      }
      files.push({
        kind: movePath && body.length === 0 ? "move" : "update",
        filePath: movePath ?? filePath,
        title: movePath ? `Moved ${filePath} -> ${movePath}` : `Patched ${filePath}`,
        diff: buildUpdatedDiff(filePath, movePath, body),
      })
      continue
    }
    i += 1
  }

  return files
}

function buildAddedDiff(filePath: string, body: string[]) {
  const normalizedPath = normalizeDiffPath(filePath)
  const lines = body.map((line) => (line.startsWith("+") ? line.slice(1) : line))
  const header = [
    `diff --git a/${normalizedPath} b/${normalizedPath}`,
    "new file mode 100644",
    "--- /dev/null",
    `+++ b/${normalizedPath}`,
    `@@ -0,0 +1,${lines.length} @@`,
  ]
  return [...header, ...lines.map((line) => `+${line}`)].join("\n")
}

function buildUpdatedDiff(filePath: string, movePath: string | null, body: string[]) {
  const previous = normalizeDiffPath(filePath)
  const next = normalizeDiffPath(movePath ?? filePath)
  const header = [`diff --git a/${previous} b/${next}`]
  if (movePath) {
    header.push(`rename from ${previous}`)
    header.push(`rename to ${next}`)
  }
  header.push(`--- a/${previous}`)
  header.push(`+++ b/${next}`)
  return body.length > 0 ? [...header, ...buildUnifiedHunks(body)].join("\n") : header.join("\n")
}

function buildUnifiedHunks(body: string[]) {
  const hunks: string[] = []
  let current: string[] = []

  const flush = () => {
    if (current.length === 0) {
      return
    }
    const normalized = normalizeApplyPatchHunkLines(current)
    const oldCount = normalized.filter((line) => !line.startsWith("+")).length
    const newCount = normalized.filter((line) => !line.startsWith("-")).length
    hunks.push(`@@ -1,${Math.max(oldCount, 0)} +1,${Math.max(newCount, 0)} @@`)
    hunks.push(...normalized)
    current = []
  }

  for (const line of body) {
    if (line.startsWith("@@")) {
      flush()
      continue
    }
    current.push(line)
  }

  flush()
  return hunks.length > 0 ? hunks : ["@@ -1,0 +1,0 @@"]
}

function normalizeApplyPatchHunkLines(lines: string[]) {
  return lines.map((line) => {
    if (!line) {
      return " "
    }
    if (line.startsWith("+")) {
      return line
    }
    if (line.startsWith("-")) {
      return line
    }
    if (line.startsWith("\\")) {
      return line
    }
    if (line.startsWith(" ")) {
      return line
    }
    return ` ${line}`
  })
}

function normalizeDiffPath(filePath: string) {
  return filePath.replace(/^\/+/, "") || filePath
}
