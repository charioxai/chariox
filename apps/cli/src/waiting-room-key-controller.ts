import { shouldHandleWaitingRoomKeyEvent } from "./hotkeys.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import {
  deriveWaitingRoomKeyNavigationDecision,
  waitingRoomSessionLifecycleActionForEvent,
  type WaitingRoomSessionLifecycleAction,
} from "./waiting-room-controller.js"

export type WaitingRoomKeyControllerEvent = {
  name: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  super?: boolean
}

export type WaitingRoomKeyControllerDeps = {
  isAttached: () => boolean
  hotkeysOpen: () => boolean
  promptFocused: () => boolean
  commandCenterOpen: () => boolean
  commandCenterQuery: () => string
  getWaitingRoomState: () => WaitingRoomState
  getSessions: () => SessionListEntry[]
  getProviderCatalog: () => ProviderCatalog
  getRemoteState: () => WaitingRoomRemoteState
  getThemeRegistry?: () => ThemeRegistry
  reconcileWaitingRoom: (state: WaitingRoomState) => void
  setWaitingRoomState: (state: WaitingRoomState) => void
  rebuildTranscript: () => void
  applyLifecycleAction: (action: WaitingRoomSessionLifecycleAction) => void
  activateWaitingRoom: () => void
}

export type WaitingRoomKeyController = {
  handleKey(event: WaitingRoomKeyControllerEvent): boolean
}

export function createWaitingRoomKeyController(
  deps: WaitingRoomKeyControllerDeps,
): WaitingRoomKeyController {
  return {
    handleKey(event) {
      const promptFocused = deps.promptFocused()
      if (!shouldHandleWaitingRoomKeyEvent(event, {
        attached: deps.isAttached(),
        hotkeysOpen: deps.hotkeysOpen(),
        promptFocused,
        commandCenterOpen: deps.commandCenterOpen(),
        commandCenterQuery: deps.commandCenterQuery(),
      })) {
        return false
      }

      const keyNavigationOptions = {
        event,
        state: deps.getWaitingRoomState(),
        sessions: deps.getSessions(),
        catalog: deps.getProviderCatalog(),
        remote: deps.getRemoteState(),
      }
      const themeRegistry = deps.getThemeRegistry?.()
      const keyNavigation = deriveWaitingRoomKeyNavigationDecision(
        themeRegistry
          ? { ...keyNavigationOptions, themeRegistry }
          : keyNavigationOptions,
      )
      if (keyNavigation.action === "navigate") {
        deps.reconcileWaitingRoom(keyNavigation.nextState)
        return true
      }
      if (keyNavigation.action === "release") {
        deps.setWaitingRoomState(keyNavigation.nextState)
        deps.rebuildTranscript()
        return true
      }
      const sessionLifecycleAction = waitingRoomSessionLifecycleActionForEvent({
        event,
        promptFocused,
      })
      if (sessionLifecycleAction) {
        deps.applyLifecycleAction(sessionLifecycleAction)
        return true
      }
      if (event.eventType !== "release" && (event.name === "return" || event.name === "enter")) {
        deps.activateWaitingRoom()
      }
      return true
    },
  }
}
