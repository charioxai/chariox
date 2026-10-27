export type AgentFocusTransitionController = {
  hasPending(): boolean
  track<T>(operation: () => Promise<T>): Promise<T>
  wait(): Promise<void>
}

export function createAgentFocusTransitionController(): AgentFocusTransitionController {
  let pendingTransition: Promise<void> | null = null

  return {
    hasPending() {
      return pendingTransition !== null
    },
    async track<T>(operation: () => Promise<T>): Promise<T> {
      const transition = operation()
      const completion = transition.then(
        () => undefined,
        () => undefined,
      )
      pendingTransition = completion
      try {
        return await transition
      } finally {
        if (pendingTransition === completion) {
          pendingTransition = null
        }
      }
    },
    async wait() {
      if (!pendingTransition) {
        return
      }
      await pendingTransition
    },
  }
}
