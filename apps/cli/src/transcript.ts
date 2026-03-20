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
  const todos = formatTodoTranscriptUpdate(update)
  if (todos) {
    return todos
  }

  const readResult = formatReadTranscriptUpdate(update)
  if (readResult) {
    return readResult
  }

  const grepResult = formatGrepTranscriptUpdate(update)
  if (grepResult) {
    return grepResult
  }

  const sections: string[] = []
  const tool = nonEmpty(update.tool) ?? "tool"
  const status = nonEmpty(update.status)
  sections.push(status && status !== "completed" ? `${tool} [${status}]` : tool)

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
    sections.push(`$ ${command}`)
  } else {
    const renderedInput = renderInput(update.input)
    if (renderedInput) {
      sections.push(renderedInput)
    }
  }

  if (text && !sections.includes(text)) {
    sections.push(text)
  }
  if (output && !sections.includes(output)) {
    sections.push(output)
  }
  if (error && !sections.includes(error)) {
    sections.push(`Error: ${error}`)
  }
  if (raw && !sections.includes(raw) && raw !== output && raw !== text) {
    sections.push(raw)
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
  const lines = [`Todos: ${remaining} ${remaining === 1 ? "todo" : "todos"} remaining`]

  for (const todo of todos) {
    const content = typeof todo.content === "string" ? todo.content.trim() : ""
    if (!content) {
      continue
    }
    lines.push(`${todo.status === "completed" ? "[✓]" : todo.status === "cancelled" ? "[-]" : "[ ]"} ${content}`)
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
  if (update.tool !== "read") {
    return null
  }

  const input = readReadInput(update.input)
  const content = readReadContent(update.output) ?? readReadContent(update.raw)
  if (!input?.filePath || !content) {
    return null
  }

  const header = `read: ${input.filePath}${formatReadWindow(input)}`
  return `${header}\n${collapseMiddleLines(content, 20)}`
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

  const parsed = parseGrepOutput(output, input.path)
  if (!parsed) {
    return null
  }

  return [`grep: ${input.pattern}${parsed.summary}`, ...parsed.lines].join("\n")
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

  const match = value.match(/<content>([\s\S]*?)<\/content>/)
  if (!match) {
    return null
  }

  return trimTrailingNewlines(match[1] ?? "")
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

  const result: { pattern: string; path?: string } = { pattern }
  if (typeof value.path === "string" && value.path.trim()) {
    result.path = value.path.trim()
  }
  return result
}

function parseGrepOutput(output: string, rootPath?: string) {
  const lines = output.split(/\r?\n/)
  const summaryLine = lines[0]?.trim() ?? ""
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
    ? ` in ${displayGrepPath(entries[0]![0], rootPath)} (${totalMatches} matches)`
    : ` (${totalMatches} matches in ${entries.length} files)`
  const renderedLines = entries.flatMap(([filePath, fileLines]) => [displayGrepPath(filePath, rootPath), ...fileLines])

  return { summary, lines: renderedLines }
}

function displayGrepPath(filePath: string, rootPath?: string) {
  if (rootPath && filePath.startsWith(`${rootPath}/`)) {
    return filePath.slice(rootPath.length + 1)
  }
  return filePath
}
