import { buildHotkeySections } from "./hotkey-help.js"
import type { SessionListEntry } from "./sessions.js"
import {
  clampSessionBrowserIndex,
  sessionBrowserVisibleSessions,
} from "@arroba/kernel-client/session-browser-policy"

export type SessionBrowserProjectionControllerDeps = {
  isAttached: () => boolean
  availableSessions: () => SessionListEntry[]
  selectedIndex: () => number
  setSelectedIndex: (index: number) => void
}

export function createSessionBrowserProjectionController(
  deps: SessionBrowserProjectionControllerDeps,
) {
  const sessions = () => sessionBrowserVisibleSessions(deps.availableSessions())
  const normalizeIndex = () => {
    const visibleSessions = sessions()
    const index = clampSessionBrowserIndex(deps.selectedIndex(), visibleSessions.length)
    if (index !== deps.selectedIndex()) {
      deps.setSelectedIndex(index)
    }
    return index
  }

  return {
    hotkeySections: () => buildHotkeySections(deps.isAttached()),
    sessions,
    normalizeIndex,
  }
}
