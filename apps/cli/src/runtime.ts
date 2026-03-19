import { LocalIpcError } from "./ipc.js"

export const DEFAULT_CONNECTED_STATUS = "Connected to the Arroba daemon."
export const MAX_TRANSIENT_POLL_FAILURES = 5
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
