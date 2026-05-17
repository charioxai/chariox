import { ARROBA_ASCII_ART, type SessionListEntry } from "./sessions.js"
import {
  catalogModelOptions,
  type BackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  DEFAULT_THEME_REGISTRY,
  themeLabel,
  type ThemeName,
  type ThemeRegistry,
} from "./theme-registry.js"
import {
  formatWaitingRoomTerminalTitle,
  waitingRoomTerminalRows,
  waitingRoomTerminals,
} from "./waiting-room-terminal-rows.js"
import {
  waitingRoomSessionRows,
  waitingRoomSessions,
  waitingRoomSessionTitleWidth,
} from "./waiting-room-session-rows.js"
import { waitingRoomRemoteRows } from "./waiting-room-remote-rows.js"
import { waitingRoomStartRows } from "./waiting-room-start-rows.js"
import {
  waitingRoomChoice,
} from "./waiting-room-choice.js"
import { cycleWaitingRoomFocusedValue } from "./waiting-room-value-cycling.js"
import {
  createWaitingRoomState,
  normalizeWaitingRoomState,
} from "./waiting-room-state.js"
import type { SliceRecord } from "./cli-types.js"

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
export {
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteKernelIsAttachable,
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachineCanDelete,
} from "./waiting-room-remote-rows.js"

export type WaitingRoomFocus =
  | "new"
  | "provider"
  | "model"
  | "effort"
  | "workspace"
  | "worktree"
  | "slice"
  | "theme"
  | "join-sessions"
  | "session"
  | "relay"
  | "machine"
  | "remote-kernel"
  | "terminal"
  | "add-terminal"

export type WaitingRoomKeyState = {
  up: boolean
  down: boolean
  left: boolean
  right: boolean
}

export type WaitingRoomState = {
  focus: WaitingRoomFocus
  sessionIndex: number
  machineIndex: number
  remoteKernelIndex: number
  terminalIndex: number
  worktreeSelectionId: string
  sliceSelectionId?: string
  providerId: BackendProviderId
  modelId: string
  effort: string
  themeId: ThemeName
  introStep: number
  keyState: WaitingRoomKeyState
}

export type WaitingRoomRemoteMachine = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name?: string
  trust_status?: "approved" | "pending" | "forgotten"
  online?: boolean
  kernel_count: number
  available_providers?: string[]
  pending?: boolean
}

export type WaitingRoomRemoteKernel = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  kernel_alias?: string | null
  relay_alias?: string | null
  available_providers?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type WaitingRoomRemoteState = {
  inventoryStatus?: "loading" | "ready" | "error"
  loadingFrame?: number
  cloudNotice?: string | null
  relay?: {
    configured: boolean
    connected: boolean
    relay_url?: string | null
  } | null
  machines?: WaitingRoomRemoteMachine[]
  kernels?: WaitingRoomRemoteKernel[]
  terminals?: WaitingRoomTerminal[]
  slices?: SliceRecord[]
}

export type WaitingRoomTerminalType = "cli" | "web" | "ios" | "android"

export type WaitingRoomTerminal = {
  terminal_id: string
  terminal_type: WaitingRoomTerminalType
  alias?: string | null
  paired_at_ms: number
  revoked: boolean
}

export type WaitingRoomTargetState = {
  workspacePath: string
  worktreePath: string
}

export type WaitingRoomRow = {
  id: string
  title: string
  value: string
  titleWidth: number
  columns?: string[]
  indent: number
  focused: boolean
  selectable: boolean
  scrollbar: string
}

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

export function waitingRoomRows(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  remote: WaitingRoomRemoteState = {},
  targets?: WaitingRoomTargetState,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
) {
  const choice = waitingRoomChoice(state, sessions, catalog, remote)
  const inventoryLoading = remote.inventoryStatus === "loading"
  const loadingText = waitingRoomLoadingText(remote.loadingFrame)
  const modelOptions = catalogModelOptions(catalog, state.providerId)
  const visibleSessions = waitingRoomSessions(sessions)
  const terminals = waitingRoomTerminals(remote)
  const terminalTitles = terminals.map(formatWaitingRoomTerminalTitle)
  const titleWidth = Math.max(
    waitingRoomSessionTitleWidth(sessions),
    ...terminalTitles.map((title) => Math.max(0, title.length)),
    "Add New Terminal".length,
  )
  const rows: WaitingRoomRow[] = waitingRoomStartRows(state, choice, {
    modelOptions,
    remote,
    ...(targets ? { targets } : {}),
    inventoryLoading,
    loadingText,
    visibleSessionCount: visibleSessions.length,
    titleWidth,
  })

  rows.push(...waitingRoomSessionRows(state, sessions, { inventoryLoading, loadingText, titleWidth }))

  rows.push(
    ...waitingRoomRemoteRows(state, remote, titleWidth),
    ...waitingRoomTerminalRows(state, remote, titleWidth),
    {
      id: "theme",
      title: "Theme",
      value: themeLabel(state.themeId, themeRegistry),
      titleWidth,
      indent: 0,
      focused: state.focus === "theme",
      selectable: true,
      scrollbar: "",
    },
  )

  return rows
}

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}

export function arrobaArtFrame(step: number) {
  const progress = Math.max(0, Math.min(step, 12))
  return ARROBA_ASCII_ART.split("\n")
    .map((line, row) =>
      [...line]
        .map((char, index) => {
          if (char === " ") {
            return " "
          }
          const threshold = Math.floor(((row * 7 + index) % 13) + progress)
          if (threshold >= 12) {
            return char
          }
          return [".", "*", "+", "#"][modulo(row + index + step, 4)]!
        })
        .join(""),
    )
    .join("\n")
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}
