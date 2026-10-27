import type { ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import {
  moveWaitingRoomManagedMachineFocus,
  waitingRoomManagedMachineFocusTargets,
} from "./waiting-room-focus-targets.js"
import { waitingRoomConfiguresNewManagedMachine } from "./waiting-room-managed-environments.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import { cycleWaitingRoomValue } from "./waiting-room-value-cycling.js"

export type WaitingRoomManagedMachineDialogKeyEvent = {
  name: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  super?: boolean
}

export type WaitingRoomManagedMachineDialogControllerDeps = {
  isOpen: () => boolean
  state: () => WaitingRoomState
  sessions: () => SessionListEntry[]
  catalog: () => ProviderCatalog
  remote: () => WaitingRoomRemoteState
  themeRegistry?: () => ThemeRegistry
  setState: (state: WaitingRoomState) => void
  openOverlay: () => void
  closeOverlay: () => void
  renderOverlay: () => void
}

export function createWaitingRoomManagedMachineDialogController(
  deps: WaitingRoomManagedMachineDialogControllerDeps,
) {
  const open = (): boolean => {
    const state = deps.state()
    if (!waitingRoomConfiguresNewManagedMachine(state.selectedMachineRef)) {
      return false
    }
    const first = waitingRoomManagedMachineFocusTargets(state, deps.remote())[0]
    deps.setState({
      ...state,
      focus: first?.focus ?? "managed-compute",
      ...(first?.managedRepositoryIndex !== undefined
        ? { managedRepositoryIndex: first.managedRepositoryIndex }
        : {}),
      ...(first?.managedProviderAccountIndex !== undefined
        ? { managedProviderAccountIndex: first.managedProviderAccountIndex }
        : {}),
    })
    deps.openOverlay()
    return true
  }

  const handleKey = (event: WaitingRoomManagedMachineDialogKeyEvent): boolean => {
    if (!deps.isOpen()) return false
    if (event.eventType === "release") return true
    const state = deps.state()
    if (state.focus === "managed-repository-root" && event.ctrl && event.name === "u") {
      deps.setState({ ...state, managedRepositoryRoot: "" })
      deps.renderOverlay()
      return true
    }
    if (event.ctrl || event.meta || event.alt || event.super) return true
    if (event.name === "return" || event.name === "enter") {
      deps.closeOverlay()
      return true
    }
    if (event.name === "up" || event.name === "down") {
      deps.setState(moveWaitingRoomManagedMachineFocus(
        deps.state(),
        event.name === "up" ? -1 : 1,
        deps.remote(),
      ))
      deps.renderOverlay()
      return true
    }
    if (event.name === "left" || event.name === "right") {
      deps.setState(cycleWaitingRoomValue(
        deps.state(),
        deps.sessions(),
        deps.catalog(),
        event.name === "left" ? -1 : 1,
        deps.themeRegistry?.(),
        deps.remote(),
      ))
      deps.renderOverlay()
      return true
    }
    if (state.focus === "managed-repository-root") {
      if (event.name === "backspace") {
        deps.setState({
          ...state,
          managedRepositoryRoot: (state.managedRepositoryRoot ?? "").slice(0, -1),
        })
        deps.renderOverlay()
        return true
      }
      const text = event.name === "space" ? " " : event.name
      if ([...text].length === 1 && !/[\u0000-\u001f\u007f]/u.test(text)) {
        deps.setState({
          ...state,
          managedRepositoryRoot: `${state.managedRepositoryRoot ?? ""}${text}`,
        })
        deps.renderOverlay()
      }
      return true
    }
    return true
  }

  return { handleKey, open }
}
