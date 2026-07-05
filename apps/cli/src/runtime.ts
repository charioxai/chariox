import { LocalIpcError } from "./ipc.js"
import {
  getProviderActivityLabel,
  getToolActivityLabel,
  isProviderIdleStatus,
  chooseVisibleActivityLabel,
} from "@arroba/kernel-client/provider-status"
import {
  sessionStatusLabel,
  type SessionStatusMode,
} from "@arroba/kernel-client/session-runtime-status"
import {
  resolveSessionStreamingAgentId,
  resolveVisibleTranscriptAgentId,
  sessionWorkingStateAfterPromptWork,
  turnCompletionDelayMs,
  type SessionStreamingAgent,
  type TurnCompletionDelayInput,
} from "@arroba/kernel-client/session-runtime-transition"

export const DEFAULT_CONNECTED_STATUS = ""
export const MAX_TRANSIENT_POLL_FAILURES = 5
export const SILENT_POLL_THRESHOLD = 8
export const STATUS_BADGE_WIDTH = Math.max("DISCONNECTED".length, "SCREENSHOTTING".length)
const POLL_RETRY_BASE_MS = 250
const POLL_RETRY_MAX_MS = 2_000

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

export type TurnCompletionDelayOptions = TurnCompletionDelayInput

export function reconcileWorkingStateFromSession(currentWorking: boolean, sessionHasPromptWork: boolean) {
  return sessionWorkingStateAfterPromptWork(currentWorking, sessionHasPromptWork)
}

export function getSessionStatusLabel(
  mode: SessionStatusMode,
  activity: string | null,
) {
  return sessionStatusLabel(mode, activity)
}

export { getProviderActivityLabel, getToolActivityLabel, isProviderIdleStatus, chooseVisibleActivityLabel, resolveVisibleTranscriptAgentId }

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
  return turnCompletionDelayMs(options)
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
