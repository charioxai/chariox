type PollRecoveryDecision = {
  retry: boolean
  delayMs: number
  message: string
}

type RunPollingLoopOptions = {
  operation: string
  intervalMs: number
  isClosing: () => boolean
  task: () => Promise<void>
  onSessionUnavailable: (error: unknown, operation: string) => unknown | Promise<unknown>
  onMarkRecovered: (operation: string, consecutiveFailures: number) => void
  onMarkDegraded: (operation: string, message: string) => void
  onFatalError: (error: unknown, operation: string) => unknown | Promise<unknown>
  formatError: (error: unknown) => string
  isSessionUnavailableError: (error: unknown) => boolean
  getPollRecoveryDecision: (
    operation: string,
    error: unknown,
    consecutiveFailures: number,
  ) => PollRecoveryDecision
  sleep?: (ms: number) => Promise<void>
  logger?: {
    warn: (message: string, fields?: Record<string, unknown>) => void
    info?: (message: string, fields?: Record<string, unknown>) => void
    error?: (message: string, fields?: Record<string, unknown>) => void
  } | null
}

type EvaluateConnectionHealthOptions = {
  attached: boolean
  working: boolean
  now: number
  lastDaemonActivityAt: number
  consecutiveSilentPolls: number
  silentThreshold: number
  silenceWindowMs: number
}

type EvaluateConnectionHealthResult = {
  nextConsecutiveSilentPolls: number
  shouldRecover: boolean
  timeSinceLastActivityMs: number
}

export async function runPollingLoop(options: RunPollingLoopOptions): Promise<void> {
  const sleep = options.sleep ?? defaultSleep
  let consecutiveFailures = 0

  while (!options.isClosing()) {
    try {
      await options.task()
      options.onMarkRecovered(options.operation, consecutiveFailures)
      consecutiveFailures = 0
    } catch (error) {
      if (options.isClosing()) {
        break
      }
      if (options.isSessionUnavailableError(error)) {
        options.logger?.info?.("session became unavailable; returning to unattached state", {
          operation: options.operation,
          error: options.formatError(error),
        })
        await options.onSessionUnavailable(error, options.operation)
        consecutiveFailures = 0
        continue
      }
      consecutiveFailures += 1
      options.logger?.warn("poll operation failed", {
        operation: options.operation,
        error: options.formatError(error),
        attempt: consecutiveFailures,
      })
      const decision = options.getPollRecoveryDecision(
        options.operation,
        error,
        consecutiveFailures,
      )
      if (decision.retry) {
        options.onMarkDegraded(options.operation, decision.message)
        await sleep(decision.delayMs)
        continue
      }
      options.logger?.error?.("poll operation became fatal", {
        operation: options.operation,
        error: options.formatError(error),
      })
      await options.onFatalError(error, options.operation)
      break
    }
    await sleep(options.intervalMs)
  }
}

export function evaluateConnectionHealth(
  options: EvaluateConnectionHealthOptions,
): EvaluateConnectionHealthResult {
  if (!options.attached || !options.working) {
    return {
      nextConsecutiveSilentPolls: 0,
      shouldRecover: false,
      timeSinceLastActivityMs: 0,
    }
  }

  const timeSinceLastActivityMs = options.now - options.lastDaemonActivityAt
  const isSilent = timeSinceLastActivityMs > options.silenceWindowMs
  const nextConsecutiveSilentPolls = isSilent ? options.consecutiveSilentPolls + 1 : 0

  return {
    nextConsecutiveSilentPolls,
    shouldRecover: nextConsecutiveSilentPolls >= options.silentThreshold,
    timeSinceLastActivityMs,
  }
}

async function defaultSleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
