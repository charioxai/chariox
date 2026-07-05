import {
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"

export type CollapsedTranscriptBlobEntry = {
  readonly role: string
  readonly text?: string | null
  readonly sourceText?: string | null
  readonly blobTitle?: string | null
  readonly blobSummary?: string | null
  readonly historyBlobId?: string | null
  readonly historyBlobLoaded?: boolean | null
  readonly historyBlobLoading?: boolean | null
  readonly historyBlobError?: string | null
}

export type CollapsedTranscriptBlobPresentation = {
  readonly headline: string
  readonly detail: string
  readonly actionLabel: string
  readonly stateLabel: string
}

export type CollapsedTranscriptBlobDescription = {
  readonly title: string
  readonly summary: string
}

const TOOL_STATUS_LABELS: Record<string, string> = {
  running: "RUNNING",
  completed: "COMPLETED",
  error: "ERROR",
  cancelled: "CANCELLED",
}

export function describeCollapsedTranscriptBlob(
  entry: CollapsedTranscriptBlobEntry,
): CollapsedTranscriptBlobDescription {
  if (entry.role === "tool") {
    return describeToolBlob(entry)
  }
  return {
    title: roleBlobTitle(entry.role),
    summary: summarizeText(entry.text),
  }
}

export function collapsedTranscriptBlobPresentation(
  entry: CollapsedTranscriptBlobEntry,
): CollapsedTranscriptBlobPresentation {
  const title = cleanText(entry.blobTitle) || roleBlobTitle(entry.role)
  const stateLabel = collapsedBlobStateLabel(entry)
  const summary = cleanText(entry.blobSummary)
  const heading = stateLabel ? `${title} · ${stateLabel}` : title

  return {
    headline: [`> ${heading}`, summary].filter(Boolean).join("  "),
    detail: collapsedBlobDetail(entry, stateLabel),
    actionLabel: collapsedBlobActionLabel(entry),
    stateLabel,
  }
}

export function roleBlobTitle(role: string): string {
  if (role === "turn_toggle") {
    return "turn"
  }
  return role
}

function describeToolBlob(entry: CollapsedTranscriptBlobEntry): CollapsedTranscriptBlobDescription {
  const parsed = parseToolTranscriptUpdate(entry.sourceText ?? entry.text ?? "")
  if (!parsed) {
    return {
      title: "tool",
      summary: summarizeText(entry.text),
    }
  }

  const tool = nonEmpty(parsed.tool) || "tool"
  const title = `${tool}${formatToolStatusLabel(parsed.status)}`

  if (tool === "apply_patch") {
    const files = readApplyPatchFiles(parsed)
    if (files.length > 0) {
      const summary = files.length === 1
        ? files[0]!.title
        : `${files.length} files`
      return { title, summary }
    }
  }

  if (tool === "read") {
    const input = readPathInput(parsed.input)
    if (input?.path) {
      return { title, summary: `${input.path}${input.window ? ` ${input.window}` : ""}` }
    }
  }

  if (tool === "todowrite") {
    const todos = readTodoCount(parsed)
    if (todos) {
      return { title, summary: `${todos.remaining} remaining of ${todos.total}` }
    }
  }

  const command = readCommand(parsed.input)
  if (command) {
    return { title, summary: `$ ${command}` }
  }

  const inferredPath = readPathInput(parsed.input)?.path ?? readPathInput(parsed.raw)?.path
  if (inferredPath) {
    return { title, summary: inferredPath }
  }

  return {
    title,
    summary: firstPresent(
      nonEmpty(parsed.description),
      nonEmpty(parsed.title),
      summarizeText(parsed.text),
      summarizeText(parsed.output),
      summarizeText(parsed.error),
      summarizeText(parsed.raw),
    ),
  }
}

function collapsedBlobStateLabel(entry: CollapsedTranscriptBlobEntry): string {
  if (entry.historyBlobError) {
    return "ERROR"
  }
  if (entry.historyBlobLoading) {
    return "LOADING"
  }
  if (entry.historyBlobLoaded) {
    return "LOADED"
  }
  if (entry.historyBlobId) {
    return "HISTORY"
  }
  return ""
}

function collapsedBlobDetail(entry: CollapsedTranscriptBlobEntry, stateLabel: string): string {
  if (entry.historyBlobError) {
    return `History blob failed to load: ${entry.historyBlobError}`
  }
  if (entry.historyBlobLoading) {
    return "Loading history blob content"
  }
  if (entry.historyBlobId) {
    return "History blob content is collapsed"
  }
  if (stateLabel) {
    return `${stateLabel.toLowerCase()} blob content`
  }
  return "Collapsed blob content"
}

function collapsedBlobActionLabel(entry: CollapsedTranscriptBlobEntry): string {
  if (entry.historyBlobLoading) {
    return "loading..."
  }
  if (entry.historyBlobError) {
    return "click to retry"
  }
  if (entry.historyBlobId) {
    return "click to load"
  }
  return "click to expand"
}

function cleanText(value: unknown): string {
  return typeof value === "string" ? value.trim() : ""
}

function nonEmpty(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : ""
}

function firstPresent(...values: Array<string | null | undefined>) {
  for (const value of values) {
    if (value && value.trim()) {
      return value.trim()
    }
  }
  return ""
}

function summarizeText(value: unknown) {
  if (typeof value !== "string") {
    return ""
  }
  const line = value
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .split("\n")
    .map((part) => part.trim())
    .find(Boolean)
  if (!line) {
    return ""
  }
  return line.length > 120 ? `${line.slice(0, 117)}...` : line
}

function formatToolStatusLabel(status: unknown) {
  if (typeof status !== "string" || !status.trim()) {
    return ""
  }
  return ` · ${(TOOL_STATUS_LABELS[status] ?? status.trim().toUpperCase())}`
}

function readCommand(input: unknown) {
  if (!input || typeof input !== "object" || !("command" in input)) {
    return null
  }
  const command = (input as { command?: unknown }).command
  return typeof command === "string" && command.trim() ? command.trim() : null
}

function readPathInput(input: unknown) {
  if (!input || typeof input !== "object") {
    return null
  }
  const record = input as Record<string, unknown>
  const path = [
    record.filePath,
    record.filepath,
    record.filename,
    record.file,
    record.path,
    record.target,
    record.destination,
    record.source,
  ].find((value) => typeof value === "string" && value.trim()) as string | undefined
  if (!path) {
    return null
  }
  const offset = typeof record.offset === "number" ? record.offset : null
  const limit = typeof record.limit === "number" ? record.limit : null
  const window = offset !== null || limit !== null
    ? `[${[
      offset !== null ? `offset=${offset}` : "",
      limit !== null ? `limit=${limit}` : "",
    ].filter(Boolean).join(", ")}]`
    : ""
  return {
    path: path.trim(),
    window,
  }
}

function readTodoCount(update: ToolTranscriptUpdate) {
  const todos = [update.input, update.output, update.raw]
    .map(readTodos)
    .find((value) => value !== null)
  if (!todos) {
    return null
  }
  return {
    total: todos.length,
    remaining: todos.filter((todo) => todo.status !== "completed" && todo.status !== "cancelled").length,
  }
}

function readTodos(value: unknown): Array<{ status?: string }> | null {
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
    return value.filter((item) => item && typeof item === "object") as Array<{ status?: string }>
  }
  if (typeof value !== "object" || !("todos" in value)) {
    return null
  }
  const todos = (value as { todos?: unknown }).todos
  return Array.isArray(todos)
    ? todos.filter((item) => item && typeof item === "object") as Array<{ status?: string }>
    : null
}
