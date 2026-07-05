import {
  sessionFocusedStatusBadge,
  type SessionAgentBusyState,
  type SessionFocusedStatusBadge,
} from "@arroba/kernel-client/session-runtime-status"

type FocusedStatusBadgeControllerDeps = {
  isAttached: () => boolean
  daemonDisconnected: () => boolean
  activeStatusLabel: () => string | null
  focusedBusy: () => boolean
  agents: () => SessionAgentBusyState[]
}

export function createFocusedStatusBadgeController(
  deps: FocusedStatusBadgeControllerDeps,
): {
  badge: () => SessionFocusedStatusBadge
} {
  const badge = (): SessionFocusedStatusBadge => {
    return sessionFocusedStatusBadge({
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
