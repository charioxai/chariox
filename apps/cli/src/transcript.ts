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
