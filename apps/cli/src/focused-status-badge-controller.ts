import {
  deriveFocusedStatusBadge,
  type AgentBusyState,
  type FocusedStatusBadge,
} from "./session-chrome-state.js"

type FocusedStatusBadgeControllerDeps = {
  isAttached: () => boolean
  daemonDisconnected: () => boolean
  activeStatusLabel: () => string | null
  focusedBusy: () => boolean
  agents: () => AgentBusyState[]
}

export function createFocusedStatusBadgeController(
  deps: FocusedStatusBadgeControllerDeps,
): {
  badge: () => FocusedStatusBadge
} {
  const badge = (): FocusedStatusBadge => {
    return deriveFocusedStatusBadge({
      attached: deps.isAttached(),
      daemonDisconnected: deps.daemonDisconnected(),
      activeStatusLabel: deps.activeStatusLabel(),
      focusedBusy: deps.focusedBusy(),
      agents: deps.agents(),
    })
  }

  return {
    badge,
  }
}
