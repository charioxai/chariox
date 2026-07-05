import type { SessionFocusedStatusBadge } from "@arroba/kernel-client/session-runtime-status"

export type StatusIndicatorControllerDeps<TBox = unknown> = {
  isAttached: () => boolean
  getBadge: () => SessionFocusedStatusBadge | null
  getAnimationFrame: () => number
  resetFocusedBadgeChange: () => void
  logFocusedBadgeChange: (badge: SessionFocusedStatusBadge) => void
  renderIndicator: (options: {
    box: TBox | undefined
    attached: boolean
    badge: SessionFocusedStatusBadge | null
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
