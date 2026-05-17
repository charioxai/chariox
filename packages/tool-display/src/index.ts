import { guessPathFenceLanguage } from "./language.js"
import type {
  ApplyPatchFile,
  ToolDisplay,
  ToolDisplayBlock,
  ToolDisplayPatchLine,
  ToolTranscriptUpdate,
} from "./types.js"

export {
  guessPathFenceLanguage,
} from "./language.js"
export type {
  ApplyPatchFile,
  InlineCodeSpan,
  ToolDisplay,
  ToolDisplayBlock,
  ToolDisplayPatchFile,
  ToolDisplayPatchLine,
  ToolDisplayStatus,
  ToolTranscriptUpdate,
} from "./types.js"
export {
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  shouldRenderProviderStatus,
  shouldSkipConsecutiveTranscriptEntry,
  splitInlineCodeSpans,
} from "./transcript-update.js"

export function formatToolTranscriptUpdate(update: ToolTranscriptUpdate) {
  for (const formatter of [
    formatApplyPatchTranscriptUpdate,
    formatTodoTranscriptUpdate,
    formatReadTranscriptUpdate,
    formatGrepTranscriptUpdate,
  ]) {
    const formatted = formatter(update)
    if (formatted) {
      return formatted
    }
  }

  const sections: string[] = []
  const tool = nonEmpty(update.tool) ?? "tool"
  const status = nonEmpty(update.status)
  sections.push(`**${tool}**${formatToolStatusBadge(status)}`)

  const title = nonEmpty(update.title)
  const description = nonEmpty(update.description)
  const text = nonEmpty(update.text)
  const output = nonEmpty(trimTrailingNewlines(renderDetail(update.output)))
  const error = nonEmpty(update.error)
  const raw = nonEmpty(renderDetail(update.raw))

  if (title && title !== description) {
    sections.push(title)
  }
  if (description) {
    sections.push(description)
  }

  const command = readCommand(update.input)
  if (command) {
    sections.push(`**Command**\n\`\`\`bash\n$ ${command}\n\`\`\``)
  } else {
    const renderedInput = renderInput(update.input)
    if (renderedInput) {
      sections.push(renderToolBlock(update, "Input", renderedInput))
    }
  }

  if (text && !sections.includes(text)) {
    sections.push(text)
  }
  if (output && !sections.includes(output)) {
    sections.push(renderToolBlock(update, "Output", output))
  }
  if (error && !sections.includes(error)) {
    sections.push(renderLabeledCodeBlock("Error", error, "text"))
  }
  if (raw && !sections.includes(raw) && raw !== output && raw !== text) {
    sections.push(renderToolBlock(update, "Details", raw))
  }

  return sections.join("\n\n")
}

export function formatToolDisplay(update: ToolTranscriptUpdate): ToolDisplay {
  const tool = nonEmpty(update.tool) ?? "tool"
  const status = nonEmpty(update.status) ?? undefined
  const markdown = formatToolTranscriptUpdate(update)
  const patchFiles = readApplyPatchFiles(update)
  const readBlock = readToolDisplayReadBlock(update)
  const blocks: ToolDisplayBlock[] = []

  if (patchFiles.length > 0) {
    blocks.push({
      kind: "patch",
      files: patchFiles.map((file) => ({
        ...file,
        previewLines: buildApplyPatchNewPreview(file),
      })),
    })
  } else if (readBlock) {
    blocks.push(readBlock)
  } else {
    const command = readCommand(update.input)
    if (command) {
      blocks.push({ kind: "code", language: "bash", text: `$ ${command}` })
    }
    const output = nonEmpty(trimTrailingNewlines(renderDetail(update.output)))
    const text = nonEmpty(update.text)
    const error = nonEmpty(update.error)
    if (text) {
      blocks.push({ kind: "text", text })
    }
    if (output) {
      blocks.push({
        kind: "code",
        language: guessToolBlockLanguage(update, "Output", output),
        text: output,
      })
    }
    if (error) {
      blocks.push({ kind: "code", language: "text", text: error })
    }
    if (blocks.length === 0 && markdown) {
      blocks.push({ kind: "text", text: markdown })
    }
  }

  const summary = summarizeToolDisplay(update, patchFiles, markdown)
  const title = nativeToolDisplayTitle(tool)
  return {
    version: 1,
    tool,
    ...(status ? { status } : {}),
    title,
    summary,
    collapsed: {
      title: `${title}${formatToolStatusBadge(status)}`,
      summary,
    },
    blocks,
  }
}

function summarizeToolDisplay(update: ToolTranscriptUpdate, patchFiles: ApplyPatchFile[], markdown: string) {
  if (patchFiles.length > 0) {
    if (patchFiles.length === 1) {
      return patchFiles[0]!.title
    }
    return `${patchFiles.length} files`
  }
  const command = readCommand(update.input)
  if (command) {
    return `$ ${command}`
  }
  const readInput = readReadInput(update.input)
  if (isNativeReadTool(update.tool) && readInput?.filePath) {
    return `${readInput.filePath}${formatReadWindow(readInput)}`
  }
  return nonEmpty(update.description)
    ?? nonEmpty(update.title)
    ?? firstLine(update.text)
    ?? plainMarkdownLine(firstLine(markdown))
    ?? firstLine(update.output)
    ?? firstLine(update.error)
    ?? ""
}

function firstLine(value: unknown) {
  if (typeof value !== "string") {
    return null
  }
  return nonEmpty(value.split(/\r?\n/)[0])
}

function plainMarkdownLine(value: string | null) {
  return value?.replace(/\*\*/g, "").replace(/`/g, "")
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

function nonEmpty(value?: string | null) {
  const normalized = value?.trim()
  return normalized ? normalized : null
}

function trimTrailingNewlines(value: string) {
  return value.replace(/[\r\n]+$/, "")
}

function renderDetail(value: unknown) {
  if (value == null) {
    return ""
  }
  if (typeof value === "string") {
    return value
  }
  return JSON.stringify(value, null, 2)
}

function readCommand(input: unknown) {
  if (!input || typeof input !== "object" || !("command" in input)) {
    return null
  }
  const command = (input as { command?: unknown }).command
  return typeof command === "string" && command.trim() ? command : null
}

function renderInput(input: unknown) {
  if (input == null) {
    return null
  }
  if (typeof input === "string") {
    return input.trim() ? input : null
  }
  if (typeof input !== "object") {
    return String(input)
  }
  if (Array.isArray(input) && input.length === 0) {
    return null
  }
  if (!Array.isArray(input) && Object.keys(input).length === 0) {
    return null
  }
  return JSON.stringify(input, null, 2)
}

type TodoItem = {
  content?: unknown
  status?: unknown
}

type ReadInput = {
  filePath?: unknown
  offset?: unknown
  limit?: unknown
}

type GrepInput = {
  pattern?: unknown
  path?: unknown
  include?: unknown
}

type WebFetchInput = {
  format?: unknown
}

const TOOL_BLOB_VISIBLE_LINES = 10

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

type ManagedIoChange = {
  path?: unknown
  kind?: unknown
  diff?: unknown
  diff_truncated?: unknown
  diffTruncated?: unknown
}

function formatTodoTranscriptUpdate(update: ToolTranscriptUpdate) {
  if (update.tool !== "todowrite") {
    return null
  }

  const todos = readTodos(update.input) ?? readTodos(update.output) ?? readTodos(update.raw)
  if (!todos) {
    return null
  }

  const remaining = todos.filter((todo) => todo.status !== "completed" && todo.status !== "cancelled").length
  const lines = [`**Todo list**${formatToolStatusBadge(nonEmpty(update.status))}`, `Remaining: ${remaining} ${remaining === 1 ? "todo" : "todos"}`]

  for (const todo of todos) {
    const content = typeof todo.content === "string" ? todo.content.trim() : ""
    if (!content) {
      continue
    }
    lines.push(`${todo.status === "completed" ? "- [x]" : todo.status === "cancelled" ? "- [-]" : "- [ ]"} ${content}`)
  }

  return lines.join("\n")
}

export function readApplyPatchFiles(update: ToolTranscriptUpdate) {
  const managedFiles = readManagedIoChangeFiles(update)
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

function formatApplyPatchTranscriptUpdate(update: ToolTranscriptUpdate) {
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

function readTodos(value: unknown) {
  if (value == null) {
    return null
  }
  if (typeof value === "string") {
    try {
      return readTodos(JSON.parse(value))
    } catch {
      return null
    }
  }
  if (Array.isArray(value)) {
    return value.filter(isTodoItem)
  }
  if (typeof value !== "object") {
    return null
  }
  if (!("todos" in value)) {
    return null
  }
  const todos = (value as { todos?: unknown }).todos
  return Array.isArray(todos) ? todos.filter(isTodoItem) : null
}

function isTodoItem(value: unknown): value is TodoItem {
  return Boolean(value) && typeof value === "object"
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

function readManagedIoChangeFiles(update: ToolTranscriptUpdate): ApplyPatchFile[] {
  if (!isManagedIoTool(update.tool)) {
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
        .map(readManagedIoChange)
        .filter((file): file is ApplyPatchFile => Boolean(file))
      if (files.length > 0) {
        return files
      }
    }
    const file = readManagedIoChange(normalized.change)
    if (file) {
      return [file]
    }
  }

  return []
}

function nativeToolDisplayTitle(tool: string) {
  const canonical = canonicalToolName(tool)
  if (canonical === "arroba.read_artifact") return "read"
  if (isManagedIoTool(tool)) return "patch"
  return tool
}

function isNativeReadTool(tool: unknown) {
  const canonical = canonicalToolName(tool)
  return canonical === "read" || canonical === "arroba.read_artifact"
}

function isManagedIoTool(tool: unknown) {
  const canonical = canonicalToolName(tool)
  return canonical === "arroba.edit_artifact"
    || canonical === "arroba.apply_patch"
    || canonical === "arroba.delete_artifact"
    || canonical === "arroba.move_artifact"
    || canonical === "arroba.write_artifact"
}

function canonicalToolName(tool: unknown) {
  if (typeof tool !== "string") {
    return ""
  }
  const normalized = tool.trim()
  const compact = normalized.replace(/[._-]/g, "").toLowerCase()
  if (compact === "arrobawriteartifact" || compact === "writeartifact") return "arroba.write_artifact"
  if (compact === "arrobaeditartifact" || compact === "editartifact") return "arroba.edit_artifact"
  if (
    compact === "arrobaapplypatch"
    || compact === "arrobapatchartifact"
    || compact === "mcparrobapatchartifact"
    || compact === "mcparrobaarrobapatchartifact"
  ) return "arroba.apply_patch"
  if (compact === "patchartifact") return "arroba.apply_patch"
  if (compact === "applypatch") return "apply_patch"
  if (compact === "arrobadeleteartifact" || compact === "deleteartifact") return "arroba.delete_artifact"
  if (compact === "arrobamoveartifact" || compact === "moveartifact") return "arroba.move_artifact"
  if (compact === "arrobareadartifact" || compact === "readartifact") return "arroba.read_artifact"
  return normalized
}

function readManagedIoChange(value: unknown): ApplyPatchFile | null {
  if (!isObjectValue(value)) {
    return null
  }
  const change = value as ManagedIoChange
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

function normalizeJsonLike(value: unknown): unknown {
  if (typeof value !== "string") {
    return value
  }
  const trimmed = value.trim()
  if (!trimmed || (!trimmed.startsWith("[") && !trimmed.startsWith("{"))) {
    return value
  }
  try {
    return JSON.parse(trimmed)
  } catch {
    return value
  }
}

function normalizeToolOutputPayload(value: unknown): unknown {
  const normalized = normalizeJsonLike(value)
  if (!isObjectValue(normalized)) {
    return normalized
  }

  if ("structuredContent" in normalized) {
    return normalizeToolOutputPayload(normalized.structuredContent)
  }

  const content = normalized.content
  if (Array.isArray(content)) {
    const text = content
      .map((entry) => isObjectValue(entry) && typeof entry.text === "string" ? entry.text : null)
      .find((entry): entry is string => Boolean(entry?.trim()))
    if (text) {
      return normalizeToolOutputPayload(text)
    }
  }

  return normalized
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

function readString(value: unknown) {
  return typeof value === "string" && value.trim() ? value : null
}

function isObjectValue(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value)
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

function formatReadTranscriptUpdate(update: ToolTranscriptUpdate) {
  if (!isNativeReadTool(update.tool)) {
    return null
  }

  const input = readReadInput(update.input)
  const content = readReadContent(update.output) ?? readReadContent(update.raw)
  if (!input?.filePath || !content) {
    return null
  }

  const header = `**read**${formatToolStatusBadge(nonEmpty(update.status))}\n\`${input.filePath}${formatReadWindow(input)}\``
  const body = truncateToolBlob(content)
  const fence = codeFence(body)
  return `${header}\n\n${fence}${guessPathFenceLanguage(input.filePath)}\n${body}\n${fence}`
}

function readToolDisplayReadBlock(update: ToolTranscriptUpdate): ToolDisplayBlock | null {
  if (!isNativeReadTool(update.tool)) {
    return null
  }
  const input = readReadInput(update.input)
  const content = readReadContent(update.output) ?? readReadContent(update.raw)
  if (!input?.filePath || content == null) {
    return null
  }
  return {
    kind: "code",
    language: guessPathFenceLanguage(input.filePath),
    text: truncateToolBlob(content),
  }
}

function formatGrepTranscriptUpdate(update: ToolTranscriptUpdate) {
  if (update.tool !== "grep") {
    return null
  }

  const input = readGrepInput(update.input)
  const output = typeof update.output === "string" ? trimTrailingNewlines(update.output) : null
  if (!input || !output) {
    return null
  }

  const parsed = parseGrepOutput(output, input)
  if (!parsed) {
    return null
  }

  return [
    `**grep**${formatToolStatusBadge(nonEmpty(update.status))}`,
    `Pattern: \`${input.pattern}\`${parsed.summary}`,
    ...parsed.blocks,
  ].join("\n")
}

function formatToolStatusBadge(status?: string | null) {
  switch (status) {
    case "running":
      return " · RUNNING"
    case "completed":
      return " · COMPLETED"
    case "error":
      return " · ERROR"
    case "cancelled":
      return " · CANCELLED"
    default:
      return status ? ` · ${status.trim().toUpperCase()}` : ""
  }
}

function renderLabeledCodeBlock(label: string, content: string, language = "text") {
  const body = truncateToolBlob(content)
  const fence = codeFence(body)
  return `**${label}**\n${fence}${language}\n${body}\n${fence}`
}

function renderToolBlock(update: ToolTranscriptUpdate, label: string, content: string) {
  const embedded = parseEmbeddedFileBlock(content)
  if (embedded) {
    return [`**${label}**`, renderPathCodeBlock(embedded.filePath, embedded.content, embedded.rootPath)].join("\n")
  }

  const filePath = inferToolFilePath(update, label)
  if (filePath) {
    return [`**${label}**`, renderPathCodeBlock(filePath, content)].join("\n")
  }

  return renderLabeledCodeBlock(label, content, guessToolBlockLanguage(update, label, content))
}

function renderPathCodeBlock(filePath: string, content: string, rootPath?: string) {
  const body = truncateToolBlob(content)
  const fence = codeFence(body)
  return [
    `\`${displayGrepPath(filePath, rootPath)}\``,
    `${fence}${guessPathFenceLanguage(filePath)}\n${body}\n${fence}`,
  ].join("\n")
}

function codeFence(content: string) {
  const matches = content.match(/`+/g) ?? []
  const width = matches.reduce((max, value) => Math.max(max, value.length), 2) + 1
  return "`".repeat(width)
}

function guessCodeFenceLanguage(value: string) {
  const trimmed = value.trim()
  if (!trimmed) {
    return "text"
  }
  if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
    return "json"
  }
  if (trimmed.includes("</") || trimmed.includes("<content>")) {
    return "xml"
  }
  if (trimmed.includes("diff --git") || /^@@ /m.test(trimmed)) {
    return "diff"
  }
  if (/^\s*(SELECT|INSERT|UPDATE|DELETE)\b/i.test(trimmed)) {
    return "sql"
  }
  return "text"
}

function guessToolBlockLanguage(update: ToolTranscriptUpdate, label: string, content: string) {
  const embedded = parseEmbeddedFileBlock(content)
  if (embedded) {
    return guessPathFenceLanguage(embedded.filePath)
  }
  const filePath = inferToolFilePath(update, label)
  if (filePath && label !== "Input") {
    return guessPathFenceLanguage(filePath)
  }
  if (label === "Input") {
    return guessInputFenceLanguage(update.input, content)
  }
  if (update.tool === "webfetch") {
    const format = readWebFetchFormat(update.input)
    if (format === "markdown") {
      return "markdown"
    }
    if (format === "html") {
      return "html"
    }
  }
  return guessCodeFenceLanguage(content)
}

function guessInputFenceLanguage(input: unknown, content: string) {
  if (typeof input === "string" && input.trim()) {
    return guessCodeFenceLanguage(content)
  }
  if (!input || typeof input !== "object") {
    return guessCodeFenceLanguage(content)
  }
  return "json"
}

function readWebFetchFormat(input: unknown) {
  if (!input || typeof input !== "object") {
    return null
  }
  const format = (input as WebFetchInput).format
  if (format === "markdown" || format === "html" || format === "text") {
    return format
  }
  return null
}


function readReadInput(input: unknown) {
  if (!input || typeof input !== "object") {
    return null
  }

  const value = input as ReadInput & { path?: unknown }
  const filePath = typeof value.filePath === "string"
    ? value.filePath.trim()
    : typeof value.path === "string"
      ? value.path.trim()
      : ""
  if (!filePath) {
    return null
  }

  const result: { filePath: string; offset?: number; limit?: number } = { filePath }
  if (typeof value.offset === "number") {
    result.offset = value.offset
  }
  if (typeof value.limit === "number") {
    result.limit = value.limit
  }

  return result
}

function formatReadWindow(input: { offset?: number; limit?: number }) {
  const parts: string[] = []
  if (input.offset !== undefined) {
    parts.push(`offset=${input.offset}`)
  }
  if (input.limit !== undefined) {
    parts.push(`limit=${input.limit}`)
  }
  return parts.length > 0 ? ` [${parts.join(", ")}]` : ""
}

function readReadContent(value: unknown) {
  if (typeof value !== "string" && !isObjectValue(value)) {
    return null
  }

  if (typeof value === "string") {
    const embedded = parseEmbeddedFileBlock(value)
    if (embedded) {
      return trimTrailingNewlines(embedded.content)
    }
  }

  const normalized = normalizeToolOutputPayload(value)
  if (!isObjectValue(normalized)) {
    return null
  }
  const directContent = readString(normalized.content_text) ?? readString(normalized.contentText)
  if (directContent != null) {
    return trimTrailingNewlines(directContent)
  }
  const structured = isObjectValue(normalized.structuredContent) ? normalized.structuredContent : null
  const structuredContent = structured
    ? readString(structured.content_text) ?? readString(structured.contentText)
    : null
  if (structuredContent != null) {
    return trimTrailingNewlines(structuredContent)
  }
  return null
}

function truncateToolBlob(text: string) {
  return collapseMiddleLines(text, TOOL_BLOB_VISIBLE_LINES)
}

function collapseMiddleLines(text: string, visibleLines: number) {
  const lines = text.split(/\r?\n/)
  if (lines.length <= visibleLines * 2 + 1) {
    return trimTrailingNewlines(text)
  }

  const headCount = visibleLines
  const tailCount = visibleLines
  return [...lines.slice(0, headCount), "...", ...lines.slice(-tailCount)].join("\n")
}

function readGrepInput(input: unknown) {
  if (!input || typeof input !== "object") {
    return null
  }

  const value = input as GrepInput
  const pattern = typeof value.pattern === "string" ? value.pattern.trim() : ""
  if (!pattern) {
    return null
  }

  const result: { pattern: string; path?: string; include?: string } = { pattern }
  if (typeof value.path === "string" && value.path.trim()) {
    result.path = value.path.trim()
  }
  if (typeof value.include === "string" && value.include.trim()) {
    result.include = value.include.trim()
  }
  return result
}

function parseGrepOutput(output: string, input: { path?: string; include?: string }) {
  const lines = output.split(/\r?\n/)
  const summaryLine = lines[0]?.trim() ?? ""
  if (/^No files found\.?$/i.test(summaryLine)) {
    return {
      summary: formatGrepSearchScope(input, 0),
      blocks: ["```text\nNo files found\n```"],
    }
  }
  const matchCount = Number(/^Found (\d+) matches?/.exec(summaryLine)?.[1] ?? "0")
  const files = new Map<string, string[]>()
  let currentFile: string | null = null

  for (const line of lines.slice(1)) {
    if (!line.trim()) {
      continue
    }
    if (!line.startsWith("  ") && line.endsWith(":")) {
      currentFile = line.slice(0, -1)
      files.set(currentFile, [])
      continue
    }
    if (currentFile) {
      files.get(currentFile)?.push(line.trim())
    }
  }

  if (files.size === 0) {
    return null
  }

  const entries = [...files.entries()]
  const totalMatches = matchCount > 0 ? matchCount : entries.reduce((sum, [, fileLines]) => sum + fileLines.length, 0)
  const summary = entries.length === 1
    ? ` in ${displayGrepPath(entries[0]![0], input.path)} (${totalMatches} matches)`
    : ` (${totalMatches} matches in ${entries.length} files)`
  const blocks = renderPathCodeBlockCollection(
    entries.map(([filePath, fileLines]) => ({
      filePath,
      content: fileLines.join("\n"),
      rootPath: input.path,
    })),
  )

  return { summary, blocks }
}

function formatGrepSearchScope(input: { path?: string; include?: string }, matches: number) {
  const location = input.path ? ` in ${displayGrepPath(input.path)}` : ""
  const include = input.include ? ` [${input.include}]` : ""
  return `${location}${include} (${matches} matches)`
}

function displayGrepPath(filePath: string, rootPath?: string) {
  if (rootPath && filePath.startsWith(`${rootPath}/`)) {
    return filePath.slice(rootPath.length + 1)
  }
  return filePath
}

type PathCodeBlock = {
  filePath: string
  content: string
  rootPath?: string | undefined
}

function renderPathCodeBlockCollection(items: PathCodeBlock[]) {
  if (items.length <= 1) {
    return items.map((item) => renderPathCodeBlock(item.filePath, item.content, item.rootPath))
  }

  const totalLines = items.reduce((sum, item) => sum + countPathCodeBlockLines(item), 0)
  if (totalLines <= TOOL_BLOB_VISIBLE_LINES * 2 + 1) {
    return items.map((item) => renderPathCodeBlock(item.filePath, item.content, item.rootPath))
  }

  const head = takePathCodeBlockSide(items, TOOL_BLOB_VISIBLE_LINES, "head")
  const tail = takePathCodeBlockSide(items, TOOL_BLOB_VISIBLE_LINES, "tail")
  return [...head, "...", ...tail]
}

function takePathCodeBlockSide(items: PathCodeBlock[], budget: number, side: "head" | "tail") {
  const ordered = side === "head" ? items : [...items].reverse()
  const blocks: string[] = []
  let remaining = budget

  for (const item of ordered) {
    if (remaining <= 0) {
      break
    }
    const lineCount = countPathCodeBlockLines(item)
    if (lineCount <= remaining) {
      blocks.push(renderPathCodeBlock(item.filePath, item.content, item.rootPath))
      remaining -= lineCount
      continue
    }
    blocks.push(renderPartialPathCodeBlock(item, remaining, side))
    remaining = 0
  }

  return side === "head" ? blocks : blocks.reverse()
}

function countPathCodeBlockLines(item: PathCodeBlock) {
  const body = truncateToolBlob(item.content)
  return 3 + body.split(/\r?\n/).length
}

function renderPartialPathCodeBlock(item: PathCodeBlock, budget: number, side: "head" | "tail") {
  if (budget <= 1) {
    return `\`${displayGrepPath(item.filePath, item.rootPath)}\``
  }

  const language = guessPathFenceLanguage(item.filePath)
  const lines = truncateToolBlob(item.content).split(/\r?\n/)
  const visible = Math.max(1, budget - 3)
  const clipped = side === "head" ? lines.slice(0, visible) : lines.slice(-visible)
  const body = clipped.join("\n")
  const fence = codeFence(body)
  return [
    `\`${displayGrepPath(item.filePath, item.rootPath)}\``,
    `${fence}${language}`,
    body,
    fence,
  ].join("\n")
}

type EmbeddedFileBlock = {
  filePath: string
  content: string
  rootPath?: string
}

function parseEmbeddedFileBlock(value: string): EmbeddedFileBlock | null {
  const pathMatch = value.match(/<path>([\s\S]*?)<\/path>/)
  const contentMatch = value.match(/<content>([\s\S]*?)<\/content>/)
  if (!pathMatch || !contentMatch) {
    return null
  }

  const filePath = trimTrailingNewlines(pathMatch[1] ?? "").trim()
  if (!filePath) {
    return null
  }

  return {
    filePath,
    content: trimTrailingNewlines(contentMatch[1] ?? ""),
  }
}

function inferToolFilePath(update: ToolTranscriptUpdate, label: string) {
  if (update.tool === "webfetch" || label === "Input") {
    return null
  }
  return readToolFilePath(update.input) ?? readToolFilePath(update.raw)
}

function readToolFilePath(value: unknown): string | null {
  if (!value) {
    return null
  }
  if (typeof value === "string") {
    return parseEmbeddedFileBlock(value)?.filePath ?? null
  }
  if (typeof value !== "object") {
    return null
  }

  const record = value as Record<string, unknown>
  for (const key of ["filePath", "filepath", "filename", "file", "target", "destination", "source"]) {
    const candidate = record[key]
    if (typeof candidate === "string" && looksLikeFilePath(candidate)) {
      return candidate.trim()
    }
  }

  const path = record.path
  if (typeof path === "string" && looksLikeFilePath(path)) {
    return path.trim()
  }

  return null
}

function looksLikeFilePath(value: string) {
  const trimmed = value.trim()
  if (!trimmed || trimmed.endsWith("/")) {
    return false
  }
  if (guessPathFenceLanguage(trimmed) !== "text") {
    return true
  }
  return trimmed.includes("/") && /\.[a-z0-9]+$/i.test(trimmed)
}
