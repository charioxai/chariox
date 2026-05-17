import type { FocusedStatusBadge } from "./session-chrome-state.js"

export type StatusIndicatorControllerDeps<TBox = unknown> = {
  isAttached: () => boolean
  getBadge: () => FocusedStatusBadge | null
  getAnimationFrame: () => number
  resetFocusedBadgeChange: () => void
  logFocusedBadgeChange: (badge: FocusedStatusBadge) => void
  renderIndicator: (options: {
    box: TBox | undefined
    attached: boolean
    badge: FocusedStatusBadge | null
    animationFrame: number
  }) => void
}

export function createStatusIndicatorController<TBox = unknown>(
  deps: StatusIndicatorControllerDeps<TBox>,
) {
  let box: TBox | undefined

  return {
    assignBox(value: TBox | undefined) {
      box = value
    },
    render() {
      const attached = deps.isAttached()
      const badge = attached ? deps.getBadge() : null
      if (!badge) {
        deps.resetFocusedBadgeChange()
      } else {
        deps.logFocusedBadgeChange(badge)
      }
      deps.renderIndicator({
        box,
        attached,
        badge,
        animationFrame: deps.getAnimationFrame(),
      })
    },
  }
}
