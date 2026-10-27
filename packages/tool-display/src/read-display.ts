import {
  codeFence,
  parseEmbeddedFileBlock,
  truncateToolBlob,
} from "./code-blocks.js"
import { guessPathFenceLanguage } from "./language.js"
import { formatToolStatusBadge } from "./status.js"
import {
  isObjectValue,
  nonEmpty,
  normalizeToolOutputPayload,
  readString,
  trimTrailingNewlines,
} from "./strings.js"
import { isNativeReadTool } from "./tool-names.js"
import type {
  ToolDisplayBlock,
  ToolTranscriptUpdate,
} from "./types.js"

type ReadInput = {
  filePath?: unknown
  offset?: unknown
  limit?: unknown
}

export function formatReadTranscriptUpdate(update: ToolTranscriptUpdate) {
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

export function readToolDisplayReadBlock(update: ToolTranscriptUpdate): ToolDisplayBlock | null {
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

export function readReadInput(input: unknown) {
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

export function formatReadWindow(input: { offset?: number; limit?: number }) {
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
