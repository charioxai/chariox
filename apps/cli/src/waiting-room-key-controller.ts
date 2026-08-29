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
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"
import { waitingRoomConfiguresNewManagedMachine } from "./waiting-room-managed-environments.js"

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
  beginProjectRename?: (projectId: string, currentName: string) => void
  restoreProject?: (projectId: string) => void
  activateWaitingRoom: () => void
  openManagedMachineDialog?: () => boolean
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
        const openedManagedMachine = (
          keyNavigation.key === "left" || keyNavigation.key === "right"
        )
          && keyNavigation.nextState.focus === "launch-machine"
          && !waitingRoomConfiguresNewManagedMachine(keyNavigationOptions.state.selectedMachineRef)
          && waitingRoomConfiguresNewManagedMachine(keyNavigation.nextState.selectedMachineRef)
        deps.reconcileWaitingRoom(keyNavigation.nextState)
        if (openedManagedMachine) {
          deps.openManagedMachineDialog?.()
        }
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
      if (event.eventType !== "release" && !promptFocused && !event.ctrl && !event.meta && !event.alt && !event.super) {
        const state = deps.getWaitingRoomState()
        const project = state.focus === "project-entry"
          ? waitingRoomProjectsForNavigation(deps.getRemoteState().projects)[state.projectIndex ?? 0]
          : null
        if (project && event.name === "e") {
          deps.beginProjectRename?.(project.id, project.name)
          return true
        }
        if (project && event.name === "r") {
          deps.restoreProject?.(project.id)
          return true
        }
      }
      if (event.eventType !== "release" && (event.name === "return" || event.name === "enter")) {
        const state = deps.getWaitingRoomState()
        if (
          state.focus === "launch-machine"
          && waitingRoomConfiguresNewManagedMachine(state.selectedMachineRef)
          && deps.openManagedMachineDialog?.()
        ) {
          return true
        }
        deps.activateWaitingRoom()
      }
      return true
    },
  }
}
