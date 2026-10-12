import { buildHotkeySections } from "./hotkey-help.js"
import type { SessionListEntry } from "./sessions.js"
import {
  clampSessionBrowserIndex,
  sessionBrowserVisibleSessions,
} from "@chariox/kernel-client/session-browser-policy"
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
  let previousIds: string[] = []
  let previousIndex: number | null = null
  const sessions = () => {
    const project = deps.selectedProject?.() ?? null
    return sessionBrowserVisibleSessions(deps.availableSessions(), {
      includeEnded: project?.status === "archived",
    }).filter((session) => !project || session.project_id === project.id)
  }
  const normalizeIndex = () => {
    const visibleSessions = sessions()
    const selectedIndex = deps.selectedIndex()
    const ids = visibleSessions.map(session => session.id)
    const changed = ids.length !== previousIds.length || ids.some((id, index) => id !== previousIds[index])
    const selectedId = previousIndex === selectedIndex ? previousIds[selectedIndex] : null
    const retainedIndex = changed && selectedId ? ids.indexOf(selectedId) : -1
    const index = clampSessionBrowserIndex(retainedIndex >= 0 ? retainedIndex : selectedIndex, visibleSessions.length)
    if (index !== deps.selectedIndex()) {
      deps.setSelectedIndex(index)
    }
    previousIds = ids
    previousIndex = index
    return index
  }

  return {
    hotkeySections: () => buildHotkeySections(deps.isAttached()),
    sessions,
    normalizeIndex,
    selectedProject: () => deps.selectedProject?.() ?? null,
  }
}
