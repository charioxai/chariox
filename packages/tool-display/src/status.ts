export function formatToolStatusBadge(status?: string | null) {
  switch (status) {
    case "running":
      return " · RUNNING"
    case "completed":
      return " · COMPLETED"
    case "error":
      return " · ERROR"
    case "cancelled":
      return " · CANCELLED"
    default:
      return status ? ` · ${status.trim().toUpperCase()}` : ""
  }
}

export const ACTIVE_STATUS_FALLBACK = "thinking"

export function shouldRenderProviderStatus(text: string): boolean {
  return !isProviderIdleStatus(text) && !isProviderThinkingStatus(text)
}

export function getProviderActivityLabel(text: string): string | null {
  const normalized = text.trim()
  if (!normalized || isProviderIdleStatus(text)) {
    return null
  }
  if (isProviderThinkingStatus(normalized)) {
    return ACTIVE_STATUS_FALLBACK
  }

  const statusMatch = normalized.match(/^OpenCode status:\s*(.+)$/i)
  const statusText = statusMatch?.[1]
  if (statusText) {
    return toPresentParticiplePhrase(statusText)
  }

  const actionMatch = normalized.match(/^OpenCode is\s+(.+?)[.!?]*$/i)
  const actionText = actionMatch?.[1]
  if (actionText) {
    return normalizeProviderActivityLabel(actionText)
  }

  return null
}

export function isProviderIdleStatus(text: string): boolean {
  return /^OpenCode is idle\.?$/i.test(text.trim())
}

export function normalizeProviderActivityLabel(value?: string | null): string | null {
  const trimmed = value?.trim().toLowerCase()
  return trimmed ? trimmed : null
}

export function toProviderPresentParticiplePhrase(value: string): string | null {
  return toPresentParticiplePhrase(value)
}

function isProviderThinkingStatus(text: string): boolean {
  return /^OpenCode is thinking\.\.\.$/i.test(text.trim())
}

function toPresentParticiplePhrase(value: string): string | null {
  const normalized = value.trim().toLowerCase().replace(/[_-]+/g, " ")
  if (!normalized) {
    return null
  }
  const words = normalized.split(/\s+/)
  const last = words.pop()
  if (!last) {
    return null
  }
  words.push(toPresentParticipleWord(last))
  return words.join(" ")
}

function toPresentParticipleWord(value: string): string {
  if (value.endsWith("ing")) {
    return value
  }
  if (value.endsWith("ie")) {
    return `${value.slice(0, -2)}ying`
  }
  if (/[^aeiou]e$/i.test(value) && !/(?:ee|oe|ye)$/i.test(value)) {
    return `${value.slice(0, -1)}ing`
  }
  if (/[aeiou][^aeiouwxy]$/i.test(value) && !/[aeiou][^aeiou][^aeiouwxy]$/i.test(value)) {
    return `${value}${value.at(-1)}ing`
  }
  return `${value}ing`
}
