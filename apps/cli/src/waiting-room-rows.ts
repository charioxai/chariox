import { catalogModelOptions, type ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { DEFAULT_THEME_REGISTRY, themeLabel, type ThemeRegistry } from "./theme-registry.js"
import { waitingRoomChoice } from "./waiting-room-choice.js"
import { waitingRoomRemoteRows } from "./waiting-room-remote-rows.js"
import {
  waitingRoomSessionRows,
  waitingRoomSessions,
  waitingRoomSessionTitleWidth,
} from "./waiting-room-session-rows.js"
import {
  waitingRoomExternalProviderSessionRows,
  waitingRoomExternalProviderSessionTitleWidth,
} from "./waiting-room-external-provider-session-rows.js"
import {
  waitingRoomSliceRows,
  waitingRoomSliceTitleWidth,
} from "./waiting-room-slice-rows.js"
import { waitingRoomStartRows } from "./waiting-room-start-rows.js"
import {
  formatWaitingRoomTerminalTitle,
  waitingRoomTerminalRows,
  waitingRoomTerminals,
} from "./waiting-room-terminal-rows.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
  WaitingRoomTargetState,
} from "./waiting-room-types.js"
import { waitingRoomProjectRows, waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"

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
  const projects = waitingRoomProjectsForNavigation(remote.projects)
  const titleWidth = Math.max(
    waitingRoomSessionTitleWidth(sessions),
    waitingRoomExternalProviderSessionTitleWidth(remote),
    waitingRoomSliceTitleWidth(remote),
    ...terminalTitles.map((title) => Math.max(0, title.length)),
    "Add New Terminal".length,
    ...projects.map((project) => project.name.length),
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

  if (projects.length > 0) {
    rows.push(...waitingRoomProjectRows(state, remote.projects, sessions, { inventoryLoading, loadingText, titleWidth }))
  } else {
    rows.push(...waitingRoomSessionRows(state, sessions, { inventoryLoading, loadingText, titleWidth }))
  }
  rows.push(...waitingRoomExternalProviderSessionRows(state, remote, { inventoryLoading, loadingText, titleWidth }))

  rows.push(
    ...waitingRoomRemoteRows(state, remote, titleWidth),
    ...waitingRoomSliceRows(state, remote, titleWidth),
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
