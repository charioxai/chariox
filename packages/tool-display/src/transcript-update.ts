import type {
  InlineCodeSpan,
  ToolTranscriptUpdate,
} from "./types.js"

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
  const placement = next.placement ?? previous?.placement
  const authority = next.authority ?? previous?.authority
  const executionLocation = next.execution_location ?? previous?.execution_location

  if (tool !== undefined) merged.tool = tool
  if (status !== undefined) merged.status = status
  if (title !== undefined) merged.title = title
  if (description !== undefined) merged.description = description
  if (text !== undefined) merged.text = text
  if (input !== undefined) merged.input = input
  if (output !== undefined) merged.output = output
  if (error !== undefined) merged.error = error
  if (raw !== undefined) merged.raw = raw
  if (placement !== undefined) merged.placement = placement
  if (authority !== undefined) merged.authority = authority
  if (executionLocation !== undefined) merged.execution_location = executionLocation

  return merged
}

export function shouldRenderProviderStatus(text: string) {
  return !/^OpenCode is (?:idle\.?|thinking\.\.\.)$/i.test(text.trim())
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

export function shouldSkipConsecutiveTranscriptEntry(
  previous: { role: string; text: string; emphasis?: string | undefined } | null | undefined,
  next: { role: string; text: string; emphasis?: string | undefined },
) {
  if (!previous) {
    return false
  }
  if (next.role !== "error" && next.role !== "notice") {
    return false
  }
  return previous.role === next.role
    && previous.text === next.text
    && previous.emphasis === next.emphasis
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
