export type PollerDegradationLogger = {
  warn: (message: string, fields?: Record<string, unknown>) => void
  info: (message: string, fields?: Record<string, unknown>) => void
}

export type PollerDegradationControllerDeps = {
  logger?: PollerDegradationLogger | null
  onTransportClosed: (message: string) => void
  onTransportRecovered: () => void
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
      degradedPollers.add(operation)
      deps.logger?.warn("poller entered degraded mode", {
        operation,
        degraded_pollers: [...degradedPollers],
      })
      deps.onTransportClosed(message)
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
        deps.onTransportRecovered()
      }
    },

    degradedOperations() {
      return [...degradedPollers]
    },
  }
}
