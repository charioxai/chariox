import {
  applyTranscriptDisplayState as sharedApplyTranscriptDisplayState,
  collapseLatestTranscriptTurn as sharedCollapseLatestTranscriptTurn,
  findVisibleTurnToggle as sharedFindVisibleTurnToggle,
  normalizeTranscriptTurnIds as sharedNormalizeTranscriptTurnIds,
  resolveVisibleTurnToggle as sharedResolveVisibleTurnToggle,
  setTranscriptBlobCollapsed as sharedSetTranscriptBlobCollapsed,
  setTranscriptTurnExpanded as sharedSetTranscriptTurnExpanded,
  stripTranscriptDisplayEntries as sharedStripTranscriptDisplayEntries,
} from "@arroba/kernel-client/transcript-display-state"
import { roleBlobTitle } from "@arroba/kernel-client/transcript-collapsed-blob"
import type { TranscriptEntry } from "./cli-types.js"
import {
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  type ToolTranscriptUpdate,
} from "./transcript.js"

const TOOL_STATUS_LABELS: Record<string, string> = {
  running: "RUNNING",
  completed: "COMPLETED",
  error: "ERROR",
  cancelled: "CANCELLED",
}

export function normalizeTranscriptTurnIds(entries: TranscriptEntry[]) {
  return sharedNormalizeTranscriptTurnIds(entries) as TranscriptEntry[]
}

export function stripTranscriptDisplayEntries(entries: TranscriptEntry[]) {
  return sharedStripTranscriptDisplayEntries(entries)
}

export function collapseLatestTranscriptTurn(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[] = [],
) {
  return sharedCollapseLatestTranscriptTurn(entries, collapsedTurnIds)
}

export function applyTranscriptDisplayState(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[] = [],
  activeTurnId: number | null = null,
) {
  return sharedApplyTranscriptDisplayState(entries, collapsedTurnIds, activeTurnId, {
    describeCollapsedBlob,
  }) as TranscriptEntry[]
}

export function setTranscriptTurnExpanded(
  entries: TranscriptEntry[],
  turnId: number,
  collapsedTurnIds: readonly number[],
  expanded: boolean,
  activeTurnId: number | null = null,
) {
  return sharedSetTranscriptTurnExpanded(entries, turnId, collapsedTurnIds, expanded, activeTurnId, {
    describeCollapsedBlob,
  }) as TranscriptEntry[]
}

export function setTranscriptBlobCollapsed(
  entries: TranscriptEntry[],
  entryId: number,
  collapsedTurnIds: readonly number[] = [],
  collapsed: boolean,
  activeTurnId: number | null = null,
) {
  return sharedSetTranscriptBlobCollapsed(entries, entryId, collapsedTurnIds, collapsed, activeTurnId, {
    describeCollapsedBlob,
  }) as TranscriptEntry[]
}

export function findVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  toggleEntryId?: number,
) {
  return sharedFindVisibleTurnToggle(entries, turnId, toggleEntryId)
}

export function resolveVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  preferredToggleEntryId?: number,
) {
  return sharedResolveVisibleTurnToggle(entries, turnId, preferredToggleEntryId)
}

function describeCollapsedBlob(entry: TranscriptEntry) {
  if (entry.role === "tool") {
    return describeToolBlob(entry)
  }
  return {
    title: roleBlobTitle(entry.role),
    summary: summarizeText(entry.text),
  }
}

function describeToolBlob(entry: TranscriptEntry) {
  const parsed = parseToolTranscriptUpdate(entry.sourceText ?? entry.text)
  if (!parsed) {
    return {
      title: "tool",
      summary: summarizeText(entry.text),
    }
  }

  const tool = nonEmpty(parsed.tool) ?? "tool"
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
