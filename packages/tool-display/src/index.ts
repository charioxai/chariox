import { guessPathFenceLanguage } from "./language.js"
import {
  buildApplyPatchNewPreview,
  formatApplyPatchTranscriptUpdate,
  readApplyPatchFiles,
} from "./patch-display.js"
import { formatToolStatusBadge } from "./status.js"
import {
  isObjectValue,
  nonEmpty,
  normalizeToolOutputPayload,
  readString,
  renderDetail,
  trimTrailingNewlines,
} from "./strings.js"
import {
  isNativeReadTool,
  nativeToolDisplayTitle,
} from "./tool-names.js"
import type {
  ApplyPatchFile,
  ToolDisplay,
  ToolDisplayBlock,
  ToolTranscriptUpdate,
} from "./types.js"

export {
  guessPathFenceLanguage,
} from "./language.js"
export {
  buildApplyPatchNewPreview,
  readApplyPatchFiles,
} from "./patch-display.js"
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
