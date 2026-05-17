import { type SessionListEntry } from "./sessions.js"
import { type ProviderCatalog } from "./provider-catalog.js"
import {
  DEFAULT_THEME_REGISTRY,
  type ThemeRegistry,
} from "./theme-registry.js"
import { cycleWaitingRoomFocusedValue } from "./waiting-room-value-cycling.js"
import {
  createWaitingRoomState,
  normalizeWaitingRoomState,
} from "./waiting-room-state.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

export {
  MAX_VISIBLE_WAITING_ROOM_SESSIONS,
  waitingRoomMenuMinWidth,
  waitingRoomMenuTrailingPadding,
  waitingRoomPreviewSessions,
} from "./waiting-room-session-rows.js"
export { moveWaitingRoomFocus } from "./waiting-room-focus-targets.js"
export {
  waitingRoomChoice,
  waitingRoomEfforts,
  waitingRoomModel,
} from "./waiting-room-choice.js"
export {
  createWaitingRoomState,
  normalizeWaitingRoomState,
} from "./waiting-room-state.js"
export { arrobaArtFrame } from "./waiting-room-art.js"
export { waitingRoomRows } from "./waiting-room-rows.js"
export {
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteKernelIsAttachable,
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachineCanDelete,
} from "./waiting-room-remote-rows.js"
export type {
  WaitingRoomFocus,
  WaitingRoomKeyState,
  WaitingRoomRemoteKernel,
  WaitingRoomRemoteMachine,
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
  WaitingRoomTargetState,
  WaitingRoomTerminal,
  WaitingRoomTerminalType,
} from "./waiting-room-types.js"

export function cycleWaitingRoomValue(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  delta: number,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
  remote: WaitingRoomRemoteState = {},
) {
  return cycleWaitingRoomFocusedValue(state, delta, {
    catalog,
    themeRegistry,
    remote,
    normalizeState: (next) => normalizeWaitingRoomState(next, sessions, catalog, themeRegistry, remote),
  })
}
