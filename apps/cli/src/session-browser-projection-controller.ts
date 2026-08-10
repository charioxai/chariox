import { buildHotkeySections } from "./hotkey-help.js"
import type { SessionListEntry } from "./sessions.js"
import {
  clampSessionBrowserIndex,
  sessionBrowserVisibleSessions,
} from "@arroba/kernel-client/session-browser-policy"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"

export type SessionBrowserProjectionControllerDeps = {
  isAttached: () => boolean
  availableSessions: () => SessionListEntry[]
  selectedIndex: () => number
  setSelectedIndex: (index: number) => void
  selectedProject?: () => WaitingRoomProjectSummary | null
}

export function createSessionBrowserProjectionController(
  deps: SessionBrowserProjectionControllerDeps,
) {
  const sessions = () => {
    const projectId = deps.selectedProject?.()?.id
    return sessionBrowserVisibleSessions(deps.availableSessions())
      .filter((session) => !projectId || session.project_id === projectId)
  }
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
    selectedProject: () => deps.selectedProject?.() ?? null,
  }
}
