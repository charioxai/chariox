import { guessPathFenceLanguage } from "./transcript-markdown.js"

export type ToolTranscriptUpdate = {
  id: string
  tool?: string
  status?: string
  title?: string
  description?: string
  text?: string
  input?: unknown
  output?: string
  error?: string
  raw?: string
}

export type InlineCodeSpan = {
  text: string
  code: boolean
}

export {
  guessPathFenceLanguage,
  normalizeMarkdownFenceInfoStrings,
  shouldRenderTranscriptAsMarkdown,
} from "./transcript-markdown.js"

export function mergeToolTranscriptUpdate(
  previous: ToolTranscriptUpdate | null,
  next: ToolTranscriptUpdate,
) {
  const merged: ToolTranscriptUpdate = { id: next.id }
  const tool = next.tool ?? previous?.tool
  const status = next.status ?? previous?.status
  const title = next.title ?? previous?.title
  const description = next.description ?? previous?.description
  const text = next.text ?? previous?.text
  const input = next.input ?? previous?.input
  const output = next.output ?? previous?.output
  const error = next.error ?? previous?.error
  const raw = next.raw ?? previous?.raw

  if (tool !== undefined) merged.tool = tool
  if (status !== undefined) merged.status = status
  if (title !== undefined) merged.title = title
  if (description !== undefined) merged.description = description
  if (text !== undefined) merged.text = text
  if (input !== undefined) merged.input = input
  if (output !== undefined) merged.output = output
  if (error !== undefined) merged.error = error
  if (raw !== undefined) merged.raw = raw

  return merged
}

export function shouldRenderProviderStatus(text: string) {
  return !/^OpenCode is idle\.?$/i.test(text.trim())
}

export function splitInlineCodeSpans(text: string): InlineCodeSpan[] {
  const spans: InlineCodeSpan[] = []
  let cursor = 0

  while (cursor < text.length) {
    const start = text.indexOf("`", cursor)
    if (start === -1) {
      break
    }
    const end = text.indexOf("`", start + 1)
    if (end === -1) {
      break
    }

    if (start > cursor) {
      spans.push({ text: text.slice(cursor, start), code: false })
    }
    spans.push({ text: text.slice(start + 1, end), code: true })
    cursor = end + 1
  }

  if (cursor < text.length || spans.length === 0) {
    spans.push({ text: text.slice(cursor), code: false })
  }

  return spans.filter((span) => span.text.length > 0)
}

export function parseToolTranscriptUpdate(chunk: string): ToolTranscriptUpdate | null {
  try {
    const parsed = JSON.parse(chunk) as Partial<ToolTranscriptUpdate>
    if (typeof parsed.id !== "string") {
      return null
    }
    return parsed as ToolTranscriptUpdate
  } catch {
    return null
  }
}

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

export type ApplyPatchFile = {
  kind: "add" | "delete" | "update" | "move"
  filePath: string
  title: string
  diff: string | null
}

type ApplyPatchInput = {
  patchText?: unknown
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
  if (update.tool !== "apply_patch") {
    return []
  }
  return parseApplyPatchText(readApplyPatchText(update.input))
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
    `**apply_patch**${formatToolStatusBadge(nonEmpty(update.status))}`,
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
  if (!input || typeof input !== "object") {
    return ""
  }
  const patchText = (input as ApplyPatchInput).patchText
  return typeof patchText === "string" ? patchText : ""
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
  if (update.tool !== "read") {
    return null
  }

  const input = readReadInput(update.input)
  const content = readReadContent(update.output) ?? readReadContent(update.raw)
  if (!input?.filePath || !content) {
    return null
  }

  const header = `**read**${formatToolStatusBadge(nonEmpty(update.status))}\n\`${input.filePath}${formatReadWindow(input)}\``
  const body = collapseMiddleLines(content, 20)
  const fence = codeFence(body)
  return `${header}\n\n${fence}${guessPathFenceLanguage(input.filePath)}\n${body}\n${fence}`
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
      return " · running"
    case "completed":
      return " · completed"
    case "error":
      return " · error"
    case "cancelled":
      return " · cancelled"
    default:
      return status ? ` · ${status}` : ""
  }
}

function renderLabeledCodeBlock(label: string, content: string, language = "text") {
  const fence = codeFence(content)
  return `**${label}**\n${fence}${language}\n${trimTrailingNewlines(content)}\n${fence}`
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
  const fence = codeFence(content)
  return [
    `\`${displayGrepPath(filePath, rootPath)}\``,
    `${fence}${guessPathFenceLanguage(filePath)}\n${trimTrailingNewlines(content)}\n${fence}`,
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

  const value = input as ReadInput
  const filePath = typeof value.filePath === "string" ? value.filePath.trim() : ""
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
  if (typeof value !== "string") {
    return null
  }

  const embedded = parseEmbeddedFileBlock(value)
  if (!embedded) {
    return null
  }

  return trimTrailingNewlines(embedded.content)
}

function collapseMiddleLines(text: string, maxLines: number) {
  const lines = text.split(/\r?\n/)
  if (lines.length <= maxLines) {
    return text
  }

  const headCount = Math.ceil(maxLines / 2)
  const tailCount = Math.floor(maxLines / 2)
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
  const blocks = entries.map(([filePath, fileLines]) => renderPathCodeBlock(filePath, fileLines.join("\n"), input.path))

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
