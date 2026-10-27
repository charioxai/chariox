import { evaluateConnectionHealth } from "./polling-effects.js"

type ConnectionHealthDecision = ReturnType<typeof evaluateConnectionHealth>

type ConnectionHealthWatchdogControllerOptions<TimerHandle> = {
  now: () => number
  intervalMs: number
  silenceWindowMs: number
  silentThreshold: number
  scheduleInterval: (callback: () => void, intervalMs: number) => TimerHandle
  clearInterval: (timer: TimerHandle) => void
  isClosing: () => boolean
  isAttached: () => boolean
  isWorking: () => boolean
  onRecover: (decision: ConnectionHealthDecision) => void
}

export type ConnectionHealthWatchdogController = {
  recordActivity(): void
  check(): void
  start(): void
  stop(): void
}

export function createConnectionHealthWatchdogController<TimerHandle>(
  options: ConnectionHealthWatchdogControllerOptions<TimerHandle>,
): ConnectionHealthWatchdogController {
  let lastActivityAt = options.now()
  let consecutiveSilentPolls = 0
  let interval: TimerHandle | undefined

  const stop = () => {
    if (interval === undefined) {
      return
    }
    options.clearInterval(interval)
    interval = undefined
  }

  const check = () => {
    const decision = evaluateConnectionHealth({
      attached: options.isAttached(),
      working: options.isWorking(),
      now: options.now(),
      lastDaemonActivityAt: lastActivityAt,
      consecutiveSilentPolls,
      silentThreshold: options.silentThreshold,
      silenceWindowMs: options.silenceWindowMs,
    })
    consecutiveSilentPolls = decision.nextConsecutiveSilentPolls
    if (!decision.shouldRecover) {
      return
    }
    options.onRecover(decision)
    consecutiveSilentPolls = 0
  }

  return {
    recordActivity() {
      lastActivityAt = options.now()
      consecutiveSilentPolls = 0
    },
    check,
    start() {
      if (interval !== undefined) {
        return
      }
      interval = options.scheduleInterval(() => {
        if (options.isClosing()) {
          stop()
          return
        }
        check()
      }, options.intervalMs)
    },
    stop,
  }
}
