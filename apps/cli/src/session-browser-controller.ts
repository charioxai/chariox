import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  clampSessionBrowserIndex,
  nextSessionBrowserIndex,
  resolveSessionBrowserKeyAction,
  type SessionBrowserKeyEvent,
} from "@arroba/kernel-client/session-browser-policy"
import type { WaitingRoomState } from "./waiting-room-types.js"
import {
  deriveWaitingRoomActivationDecision,
  type WaitingRoomLaunchConfig,
  type WaitingRoomSessionLifecycleAction,
} from "./waiting-room-controller.js"

type FooterTone = "info" | "error"

export type SessionBrowserControllerDeps = {
  isOpen: () => boolean
  visibleSessions: () => SessionListEntry[]
  availableSessions: () => SessionListEntry[]
  normalizeSelectedIndex: () => number
  setSelectedIndex: (updater: (index: number) => number) => void
  waitingRoomState: () => WaitingRoomState
  providerCatalog: () => ProviderCatalog
  currentProvider: () => BackendProviderId
  currentModel: () => string
  closeDialog: () => void
  renderOverlay: () => void
  flashFooter: (message: string, tone: FooterTone) => void
  attachSession: (
    session: SessionListEntry,
    createNew: boolean,
    launch: WaitingRoomLaunchConfig,
  ) => Promise<unknown>
  applyLifecycleAction: (
    action: WaitingRoomSessionLifecycleAction,
    stateOverride: WaitingRoomState,
  ) => Promise<void>
  formatError?: (error: unknown) => string
}

export function createSessionBrowserController(deps: SessionBrowserControllerDeps) {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const selectedWaitingRoomState = (selectedIndex: number): WaitingRoomState => ({
    ...deps.waitingRoomState(),
    focus: "session",
    sessionIndex: selectedIndex,
  })

  const handleKey = (event: SessionBrowserKeyEvent) => {
    const open = deps.isOpen()
    const sessions = open ? deps.visibleSessions() : []
    const selectedIndex = open ? deps.normalizeSelectedIndex() : 0
    const action = resolveSessionBrowserKeyAction({
      open,
      event,
      sessionCount: sessions.length,
      selectedIndex,
    })

    if (action.action === "ignore") {
      return false
    }
    if (action.action === "close") {
      deps.closeDialog()
      return true
    }
    if (action.action === "move") {
      if (sessions.length > 0) {
        deps.setSelectedIndex((index) => nextSessionBrowserIndex(index, action.delta, sessions.length))
        deps.renderOverlay()
      }
      return true
    }
    if (action.action === "empty") {
      deps.flashFooter("no sessions available", "error")
      return true
    }
    if (action.action === "submit") {
      const decision = deriveWaitingRoomActivationDecision({
        state: selectedWaitingRoomState(action.selectedIndex),
        sessions: deps.availableSessions(),
        catalog: deps.providerCatalog(),
        currentProvider: deps.currentProvider(),
        currentModel: deps.currentModel(),
      })
      if (decision.action !== "join") {
        deps.flashFooter(decision.action === "error" ? decision.message : "select a session to join", "error")
        return true
      }
      deps.closeDialog()
      void deps.attachSession(decision.session, false, decision.launch).then(
        () => deps.flashFooter(`attached to session ${decision.session.alias ?? decision.session.id}`, "info"),
        (error) => deps.flashFooter(formatError(error), "error"),
      )
      return true
    }
    if (action.action === "lifecycle") {
      void deps.applyLifecycleAction(
        action.lifecycleAction,
        selectedWaitingRoomState(action.selectedIndex),
      ).then(() => {
        const nextLength = deps.visibleSessions().length
        if (nextLength === 0) {
          deps.closeDialog()
        } else {
          deps.setSelectedIndex((index) => clampSessionBrowserIndex(index, nextLength))
          deps.renderOverlay()
        }
      })
      return true
    }
    return true
  }

  return {
    handleKey,
  }
}
