import {
  parseEmbeddedFileBlock,
  renderLabeledCodeBlock,
  renderPathCodeBlock,
} from "./code-blocks.js"
import { formatGrepTranscriptUpdate } from "./grep-display.js"
import { guessPathFenceLanguage } from "./language.js"
import {
  buildApplyPatchNewPreview,
  formatApplyPatchTranscriptUpdate,
  readApplyPatchFiles,
} from "./patch-display.js"
import {
  formatReadTranscriptUpdate,
  formatReadWindow,
  readReadInput,
  readToolDisplayReadBlock,
} from "./read-display.js"
import { formatToolStatusBadge } from "./status.js"
import {
  nonEmpty,
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
  sections.push(`**${tool}**${formatToolPlacementBadge(update.placement)}${formatToolStatusBadge(status)}`)

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
  const title = toolDisplayTitleWithPlacement(nativeToolDisplayTitle(tool), update.placement)
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

function toolDisplayTitleWithPlacement(title: string, placement: unknown) {
  const label = toolPlacementLabel(placement)
  return label ? `${label} · ${title}` : title
}

function formatToolPlacementBadge(placement: unknown) {
  const label = toolPlacementLabel(placement)
  return label ? ` · ${label.toUpperCase()}` : ""
}

function toolPlacementLabel(placement: unknown) {
  if (placement === "home-proxy" || placement === "worker-local" || placement === "skill snapshot") {
    return placement
  }
  return null
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

type WebFetchInput = {
  format?: unknown
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
