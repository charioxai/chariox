export type PollerDegradationLogger = {
  warn: (message: string, fields?: Record<string, unknown>) => void
  info: (message: string, fields?: Record<string, unknown>) => void
}

export type PollerDegradationControllerDeps = {
  connectedStatusLine: string
  logger?: PollerDegradationLogger | null
  setDaemonDisconnected: (value: boolean) => void
  setStatusLine: (value: string) => void
  updateSessionChrome: () => void
  appendNotice: (message: string, tone?: "warning" | "muted") => void
}

export type PollerDegradationController = {
  markDegraded(operation: string, message: string): void
  markRecovered(operation: string, failureCount: number): void
  degradedOperations(): string[]
}

export function createPollerDegradationController(
  deps: PollerDegradationControllerDeps,
): PollerDegradationController {
  const degradedPollers = new Set<string>()

  return {
    markDegraded(operation, message) {
      const wasHealthy = degradedPollers.size === 0
      degradedPollers.add(operation)
      deps.setDaemonDisconnected(true)
      deps.logger?.warn("poller entered degraded mode", {
        operation,
        degraded_pollers: [...degradedPollers],
      })
      deps.setStatusLine(message)
      deps.updateSessionChrome()
      if (wasHealthy) {
        deps.appendNotice(message, "warning")
      }
    },

    markRecovered(operation, failureCount) {
      if (failureCount === 0) {
        return
      }
      const wasDegraded = degradedPollers.delete(operation)
      if (wasDegraded) {
        deps.logger?.info("poller recovered", {
          operation,
          degraded_pollers: [...degradedPollers],
          prior_failures: failureCount,
        })
      }
      if (wasDegraded && degradedPollers.size === 0) {
        deps.setDaemonDisconnected(false)
        deps.setStatusLine(deps.connectedStatusLine)
        deps.updateSessionChrome()
        deps.appendNotice("Reconnected to the Arroba daemon.")
      }
    },

    degradedOperations() {
      return [...degradedPollers]
    },
  }
}
