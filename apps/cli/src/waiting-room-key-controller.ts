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
  openProjectEnvironment?: (projectId: string) => void
  pinProject?: (projectId: string, pinned: boolean) => void
  beginNewProject?: () => void
  beginProjectRename?: (projectId: string, currentName: string) => void
  restoreProject?: (projectId: string) => void
  activateWaitingRoom: () => void
  focusPrompt: () => void
  openManagedMachineDialog?: () => boolean
}

export type WaitingRoomKeyController = {
  handleKey(event: WaitingRoomKeyControllerEvent): boolean
  focusPromptOnSlash(event: WaitingRoomKeyControllerEvent): boolean
}

export function createWaitingRoomKeyController(
  deps: WaitingRoomKeyControllerDeps,
): WaitingRoomKeyController {
  return {
    // Runs from the renderer's key stream, before it hands the key to the
    // focused input: the waiting room has no focused prompt, so the slash and
    // the rest of the command would otherwise be dropped.
    focusPromptOnSlash(event) {
      if (deps.promptFocused() || event.eventType === "release" || event.name !== "/"
        || event.ctrl || event.meta || event.alt || event.super) {
        return false
      }
      if (!shouldHandleWaitingRoomKeyEvent(event, {
        attached: deps.isAttached(),
        hotkeysOpen: deps.hotkeysOpen(),
        promptFocused: false,
        commandCenterOpen: deps.commandCenterOpen(),
        commandCenterQuery: deps.commandCenterQuery(),
      })) {
        return false
      }
      deps.focusPrompt()
      return true
    },

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
      if (event.eventType !== "release" && !promptFocused && !event.ctrl && !event.meta && !event.alt && !event.super) {
        const state = deps.getWaitingRoomState()
        const project = state.focus === "project-entry"
          ? waitingRoomProjectsForNavigation(deps.getRemoteState().projects)[state.projectIndex ?? 0]
          : null
        if (project && event.name === "p") {
          deps.pinProject?.(project.id, !project.pinned)
          return true
        }
        if (project && event.name === "v") {
          deps.openProjectEnvironment?.(project.id)
          return true
        }
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
        if (state.focus === "project" && state.projectSelectionId === "new") {
          deps.beginNewProject?.()
          return true
        }
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
