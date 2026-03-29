import { LocalIpcError } from "./ipc.js"

export const DEFAULT_CONNECTED_STATUS = ""
export const MAX_TRANSIENT_POLL_FAILURES = 5
export const ACTIVE_STATUS_FALLBACK = "thinking"
export const STATUS_BADGE_WIDTH = Math.max("DISCONNECTED".length, "SCREENSHOTTING".length)
const POLL_RETRY_BASE_MS = 250
const POLL_RETRY_MAX_MS = 2_000

const TOOL_ACTIVITY_LABELS: Record<string, string> = {
  apply_patch: "patching",
  read: "reading",
}

export type PollRecoveryDecision = {
  retry: boolean
  delayMs: number
  message: string
}

export type ExitCleanupDecision = {
  exit: boolean
  exitCode: number
  message: string
}

export function reconcileWorkingStateFromSession(currentWorking: boolean, sessionHasPromptWork: boolean) {
  return sessionHasPromptWork ? true : currentWorking
}

export function getSessionStatusLabel(
  mode: "idle" | "working" | "disconnected",
  activity: string | null,
) {
  if (mode === "disconnected") {
    return "DISCONNECTED"
  }
  if (mode === "idle") {
    return "IDLE"
  }
  return formatBadgeLabel(normalizeActivityLabel(activity) ?? ACTIVE_STATUS_FALLBACK)
}

export function getProviderActivityLabel(text: string) {
  const normalized = text.trim()
  if (!normalized || /^OpenCode is idle\.?$/i.test(normalized)) {
    return null
  }
  if (/^OpenCode is thinking\.\.\.$/i.test(normalized)) {
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
    return normalizeActivityLabel(actionText)
  }

  return null
}

export function getToolActivityLabel(tool?: string | null) {
  const normalized = normalizeActivityLabel(tool)
  if (!normalized) {
    return null
  }
  return TOOL_ACTIVITY_LABELS[normalized] ?? toPresentParticiplePhrase(normalized)
}

export function chooseVisibleActivityLabel(
  providerActivity: string | null,
  activeToolActivity: string | null,
) {
  return activeToolActivity ?? providerActivity
}

export function shouldEndSessionOnCliExit(_createdSession: boolean, _connectedClientCount: number): boolean {
  return false
}

export function describeCliError(error: unknown): string {
  if (error instanceof LocalIpcError || error instanceof Error) {
    return error.message
  }
  return String(error)
}

export function getPollRecoveryDecision(
  operation: string,
  error: unknown,
  consecutiveFailures: number,
): PollRecoveryDecision {
  if (!(error instanceof LocalIpcError)) {
    return {
      retry: false,
      delayMs: 0,
      message: describeCliError(error),
    }
  }

  if (consecutiveFailures >= MAX_TRANSIENT_POLL_FAILURES) {
    return {
      retry: false,
      delayMs: 0,
      message: `Lost connection while ${operation}: ${describeCliError(error)}`,
    }
  }

  return {
    retry: true,
    delayMs: Math.min(POLL_RETRY_BASE_MS * 2 ** (consecutiveFailures - 1), POLL_RETRY_MAX_MS),
    message: `Lost connection while ${operation}; retrying (${consecutiveFailures}/${MAX_TRANSIENT_POLL_FAILURES - 1}).`,
  }
}

export function getExitCleanupDecision(
  error: unknown,
  previousCleanupFailure: boolean,
): ExitCleanupDecision {
  const message = describeCliError(error)

  if (previousCleanupFailure) {
    return {
      exit: true,
      exitCode: 1,
      message: `Exit cleanup failed again: ${message}. Forcing exit.`,
    }
  }

  return {
    exit: false,
    exitCode: 1,
    message: `Exit cleanup failed: ${message}. Run /exit or press Ctrl+C again to force quit.`,
  }
}

function normalizeActivityLabel(value?: string | null) {
  const trimmed = value?.trim().toLowerCase()
  return trimmed ? trimmed : null
}

function formatBadgeLabel(value: string) {
  return value.trim().toUpperCase()
}

function toPresentParticiplePhrase(value: string) {
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

function toPresentParticipleWord(value: string) {
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
