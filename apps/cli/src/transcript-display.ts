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
  let activeTurnId: number | undefined
  let nextTurnId = 1

  return entries.map((entry) => {
    const next: TranscriptEntry = { ...entry }
    if (entry.role === "user") {
      activeTurnId = entry.turnId ?? nextTurnId
      next.turnId = activeTurnId
      nextTurnId = Math.max(nextTurnId, activeTurnId + 1)
      return next
    }
    if (activeTurnId !== undefined) {
      next.turnId = activeTurnId
    }
    return next
  })
}

export function stripTranscriptDisplayEntries(entries: TranscriptEntry[]) {
  return entries.filter((entry) => entry.role !== "turn_toggle")
}

export function collapseLatestTranscriptTurn(
  entries: TranscriptEntry[],
  collapsedTurnIds: readonly number[] = [],
) {
  const nextCollapsedTurnIds = new Set(collapsedTurnIds)
  const normalized = normalizeTranscriptTurnIds(stripTranscriptDisplayEntries(entries))
  const turnIds = [...new Set(normalized.map((entry) => entry.turnId).filter((turnId): turnId is number => typeof turnId === "number"))]
  const latestTurnId = turnIds.at(-1)
  if (latestTurnId === undefined) {
    return sortedTurnIds(nextCollapsedTurnIds)
  }

  const turnEntries = normalized.filter((entry) => entry.turnId === latestTurnId)
  const finalSummary = [...turnEntries].reverse().find((entry) => entry.role === "assistant")
  if (!finalSummary || !hasCollapsibleTurnBody(turnEntries, finalSummary.id)) {
    return sortedTurnIds(nextCollapsedTurnIds)
  }

  nextCollapsedTurnIds.add(latestTurnId)
  return sortedTurnIds(nextCollapsedTurnIds)
}

export function applyTranscriptDisplayState(
  entries: TranscriptEntry[],
  expandedTurnIds: readonly number[] = [],
  activeTurnId: number | null = null,
) {
  const normalized = normalizeTranscriptTurnIds(stripTranscriptDisplayEntries(entries)).map((entry) => ({
    ...entry,
    hidden: false,
  }))
  const collapsedTurnIdSet = new Set(expandedTurnIds)
  let nextId = normalized.reduce((max, entry) => Math.max(max, entry.id), 0)
  const turnIds = [...new Set(normalized.map((entry) => entry.turnId).filter((turnId): turnId is number => typeof turnId === "number"))]

  for (const turnId of turnIds) {
    const turnEntries = normalized.filter((entry) => entry.turnId === turnId)
    const finalSummary = [...turnEntries].reverse().find((entry) => entry.role === "assistant")
    const collapsibleTurn = Boolean(finalSummary)
      && turnId !== activeTurnId
      && hasCollapsibleTurnBody(turnEntries, finalSummary!.id)
    const expanded = collapsibleTurn ? !collapsedTurnIdSet.has(turnId) : false

    for (const entry of turnEntries) {
      const blobCollapsible = computeBlobCollapsible(entry, finalSummary?.id ?? null)
      if (blobCollapsible) {
        entry.blobCollapsible = true
        entry.blobCollapsed = entry.blobCollapsed ?? true
        const preview = describeCollapsedBlob(entry)
        entry.blobTitle = preview.title
        entry.blobSummary = preview.summary
      } else {
        entry.blobCollapsible = false
        delete entry.blobCollapsed
        delete entry.blobTitle
        delete entry.blobSummary
      }
      if (!collapsibleTurn || expanded) {
        entry.hidden = false
        continue
      }
      entry.hidden = !(entry.role === "user" || entry.id === finalSummary!.id)
    }

    if (!collapsibleTurn) {
      continue
    }

    const promptIndex = normalized.findIndex((entry) => entry.turnId === turnId && entry.role === "user")
    const anchorIndex = promptIndex >= 0
      ? promptIndex
      : normalized.findIndex((entry) => entry.turnId === turnId)
    if (anchorIndex === -1) {
      continue
    }

    normalized.splice(anchorIndex + 1, 0, {
      id: ++nextId,
      role: "turn_toggle",
      text: expanded ? "click to collapse" : "click to expand",
      turnId,
      hidden: false,
      toggleMode: expanded ? "collapse" : "expand",
      blobCollapsible: false,
    })
  }

  return normalized
}

export function setTranscriptTurnExpanded(
  entries: TranscriptEntry[],
  turnId: number,
  expandedTurnIds: readonly number[],
  expanded: boolean,
  activeTurnId: number | null = null,
) {
  const nextExpandedTurnIds = new Set(expandedTurnIds)
  if (expanded) {
    nextExpandedTurnIds.delete(turnId)
  } else {
    nextExpandedTurnIds.add(turnId)
  }
  return applyTranscriptDisplayState(entries, [...nextExpandedTurnIds].sort((left, right) => left - right), activeTurnId)
}

export function setTranscriptBlobCollapsed(
  entries: TranscriptEntry[],
  entryId: number,
  expandedTurnIds: readonly number[] = [],
  collapsed: boolean,
  activeTurnId: number | null = null,
) {
  const updated = stripTranscriptDisplayEntries(entries).map((entry) => {
    if (entry.id !== entryId) {
      return { ...entry }
    }
    return {
      ...entry,
      blobCollapsed: collapsed,
    }
  })
  return applyTranscriptDisplayState(updated, expandedTurnIds, activeTurnId)
}

export function findVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  toggleEntryId?: number,
) {
  if (!turnId) {
    return undefined
  }
  return entries.find((entry) => {
    if (!entry || entry.turnId !== turnId || entry.role !== "turn_toggle" || entry.hidden) {
      return false
    }
    return toggleEntryId === undefined || entry.id === toggleEntryId
  })
}

export function resolveVisibleTurnToggle(
  entries: TranscriptEntry[],
  turnId: number | null | undefined,
  preferredToggleEntryId?: number,
) {
  return findVisibleTurnToggle(entries, turnId, preferredToggleEntryId)
    ?? findVisibleTurnToggle(entries, turnId)
}

function computeBlobCollapsible(entry: TranscriptEntry, _finalSummaryId: number | null) {
  if (entry.role === "user" || entry.role === "reasoning" || entry.role === "turn_toggle" || entry.role === "assistant") {
    return false
  }
  return entry.role === "tool" || entry.role === "error" || entry.role === "status" || entry.role === "notice"
}

function hasCollapsibleTurnBody(turnEntries: TranscriptEntry[], finalSummaryId: number) {
  return turnEntries.some((entry) => entry.role !== "user" && entry.id !== finalSummaryId)
}

function sortedTurnIds(turnIds: Iterable<number>) {
  return [...turnIds].sort((left, right) => left - right)
}

function describeCollapsedBlob(entry: TranscriptEntry) {
  if (entry.role === "tool") {
    return describeToolBlob(entry)
  }
  const title = entry.role === "assistant"
    ? "assistant"
    : entry.role === "error"
      ? "error"
      : entry.role === "status"
        ? "status"
        : "note"
  return {
    title,
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
