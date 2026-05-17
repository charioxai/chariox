import type { FocusedStatusBadge } from "./session-chrome-state.js"

export type StatusIndicatorControllerDeps = {
  isAttached: () => boolean
  getBadge: () => FocusedStatusBadge | null
  getAnimationFrame: () => number
  resetFocusedBadgeChange: () => void
  logFocusedBadgeChange: (badge: FocusedStatusBadge) => void
  renderIndicator: (options: {
    attached: boolean
    badge: FocusedStatusBadge | null
    animationFrame: number
  }) => void
}

export function createStatusIndicatorController(
  deps: StatusIndicatorControllerDeps,
) {
  return {
    render() {
      const attached = deps.isAttached()
      const badge = attached ? deps.getBadge() : null
      if (!badge) {
        deps.resetFocusedBadgeChange()
      } else {
        deps.logFocusedBadgeChange(badge)
      }
      deps.renderIndicator({
        attached,
        badge,
        animationFrame: deps.getAnimationFrame(),
      })
    },
  }
}
