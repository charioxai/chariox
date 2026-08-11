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
    const project = deps.selectedProject?.() ?? null
    return sessionBrowserVisibleSessions(deps.availableSessions(), {
      includeEnded: project?.status === "archived",
    }).filter((session) => !project || session.project_id === project.id)
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
