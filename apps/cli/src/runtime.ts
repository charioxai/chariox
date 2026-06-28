import { LocalIpcError } from "./ipc.js"
import {
  ACTIVE_STATUS_FALLBACK,
  getProviderActivityLabel,
  isProviderIdleStatus,
  normalizeProviderActivityLabel,
  toProviderPresentParticiplePhrase,
} from "@arroba/kernel-client/provider-status"
import {
  resolveSessionStreamingAgentId,
  type SessionStreamingAgent,
} from "@arroba/kernel-client/shell-agent-activity"

export const DEFAULT_CONNECTED_STATUS = ""
export const MAX_TRANSIENT_POLL_FAILURES = 5
export const SILENT_POLL_THRESHOLD = 8
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

export type TurnCompletionDelayOptions = {
  sessionHasPromptWork: boolean
  pendingTerminalRecordCount: number
  pendingTerminalRecordFlush: boolean
  lastTurnActivityAt: number
  now: number
  quietWindowMs: number
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

export { getProviderActivityLabel, isProviderIdleStatus }

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

export function resolveVisibleTranscriptAgentId(
  splitMode: boolean,
  primaryAgentId: string | null,
  focusedAgentId: string | null,
) {
  return splitMode ? (primaryAgentId ?? focusedAgentId) : focusedAgentId
}

export function resolveStreamingAgentId(
  agents: ReadonlyArray<SessionStreamingAgent>,
  activePromptTargetAgentId: string | null,
  sessionHasPromptWork: boolean,
  currentWorking: boolean,
  previousStreamingAgentId: string | null,
  useLegacyProcessingState = true,
) {
  return resolveSessionStreamingAgentId(
    agents,
    activePromptTargetAgentId,
    sessionHasPromptWork,
    currentWorking,
    previousStreamingAgentId,
    useLegacyProcessingState,
  )
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

export function getTurnCompletionDelayMs(options: TurnCompletionDelayOptions) {
  if (
    options.sessionHasPromptWork
    || options.pendingTerminalRecordCount > 0
    || options.pendingTerminalRecordFlush
  ) {
    return null
  }
  return Math.max(0, options.quietWindowMs - Math.max(0, options.now - options.lastTurnActivityAt))
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
  return normalizeProviderActivityLabel(value)
}

function formatBadgeLabel(value: string) {
  return value.trim().toUpperCase()
}

function toPresentParticiplePhrase(value: string) {
  return toProviderPresentParticiplePhrase(value)
}
