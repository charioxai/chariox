import { ARROBA_ASCII_ART, type SessionListEntry } from "./sessions.js"
import {
  BACKEND_PROVIDER_IDS,
  backendProviderLabel,
  catalogModelOptions,
  normalizeBackendProviderId,
  selectConfiguredModel,
  selectConfiguredVariant,
  type BackendProviderId,
  type CatalogModelOption,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  DEFAULT_THEME_REGISTRY,
  normalizeThemeName,
  themeLabel,
  themeOptions,
  type ThemeName,
  type ThemeRegistry,
} from "./theme-registry.js"
import {
  cycleWaitingRoomWorktreeSelectionId,
  describeWaitingRoomWorktreeSelection,
  normalizeWaitingRoomWorktreeSelectionId,
} from "./waiting-room-worktrees.js"
import {
  formatWaitingRoomTerminalTitle,
  waitingRoomTerminalRows,
  waitingRoomTerminals,
} from "./waiting-room-terminal-rows.js"
import {
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachines,
  waitingRoomRemoteRows,
} from "./waiting-room-remote-rows.js"
import type { SliceRecord } from "./cli-types.js"

export {
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteKernelIsAttachable,
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachineCanDelete,
} from "./waiting-room-remote-rows.js"

export const MAX_VISIBLE_WAITING_ROOM_SESSIONS = 2
const WAITING_ROOM_ROW_TITLE_MIN_WIDTH = 24
const WAITING_ROOM_STATUS_MIN_WIDTH = "Status".length
const WAITING_ROOM_TIMESTAMP_MIN_WIDTH = "0000-00-00 00:00 UTC".length
const WAITING_ROOM_MENU_TRAILING_PADDING = 2

function formatWaitingRoomSessionTitle(session: SessionListEntry) {
  const label = session.alias ? `${session.id} (${session.alias})` : session.id
  return sessionHasActiveWork(session) ? `* ${label}` : label
}

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

export function createWaitingRoomState(
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  providerId: BackendProviderId,
  model: string,
  effort: string,
  themeId: unknown = "opencode",
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
): WaitingRoomState {
  const selected = selectConfiguredModel(catalog, model, providerId)
  return normalizeWaitingRoomState(
    {
      focus: "new",
      sessionIndex: 0,
      machineIndex: 0,
      remoteKernelIndex: 0,
      terminalIndex: 0,
      worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(),
      sliceSelectionId: "none",
      providerId,
      modelId: selected?.id ?? model,
      effort: selectConfiguredVariant(selected, effort),
      themeId: normalizeThemeName(themeId, themeRegistry),
      introStep: 0,
      keyState: { up: false, down: false, left: false, right: false },
    },
    sessions,
    catalog,
    themeRegistry,
  )
}

export function normalizeWaitingRoomState(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
  remote: WaitingRoomRemoteState = {},
) {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const terminals = waitingRoomTerminals(remote)
  const slices = waitingRoomSlices(remote)
  const providerId = normalizeBackendProvider(state.providerId)
  const selected = selectConfiguredModel(catalog, state.modelId, providerId)
  const efforts = waitingRoomEfforts(selected)
  const focus = (visibleSessions.length === 0 && (state.focus === "session" || state.focus === "join-sessions"))
    ? "new"
    : previewSessions.length === 0 && state.focus === "session"
      ? "join-sessions"
    : remoteMachines.length === 0 && state.focus === "machine"
      ? "relay"
      : remoteKernels.length === 0 && state.focus === "remote-kernel"
        ? "relay"
        : terminals.length === 0 && state.focus === "terminal"
          ? "add-terminal"
        : slices.length === 0 && state.focus === "slice"
          ? "worktree"
        : state.focus
  return {
    ...state,
    focus,
    providerId,
    sessionIndex: visibleSessions.length === 0 ? 0 : modulo(state.sessionIndex, visibleSessions.length),
    machineIndex: remoteMachines.length === 0 ? 0 : modulo(state.machineIndex, remoteMachines.length),
    remoteKernelIndex: remoteKernels.length === 0 ? 0 : modulo(state.remoteKernelIndex, remoteKernels.length),
    terminalIndex: terminals.length === 0 ? 0 : modulo(state.terminalIndex, terminals.length),
    worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(state.worktreeSelectionId),
    sliceSelectionId: normalizeWaitingRoomSliceSelectionId(state.sliceSelectionId, slices),
    modelId: selected?.id ?? state.modelId,
    effort: efforts.includes(state.effort) ? state.effort : efforts[0] ?? "",
    themeId: normalizeThemeName(state.themeId, themeRegistry),
  }
}

export function waitingRoomModel(state: WaitingRoomState, catalog: ProviderCatalog) {
  return catalogModelOptions(catalog, state.providerId).find((option) => option.id === state.modelId) ?? null
}

export function waitingRoomEfforts(option: CatalogModelOption | null) {
  if (!option || option.variants.length === 0) {
    return [""]
  }
  return option.variants
}

export function moveWaitingRoomFocus(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  delta: number,
  remote: WaitingRoomRemoteState = {},
) {
  const order = waitingRoomFocusTargets(sessions, remote)
  const currentIndex = Math.max(
    0,
    order.findIndex((target) => (
      target.focus === state.focus
      && (target.focus !== "session" || target.sessionIndex === state.sessionIndex)
      && (target.focus !== "machine" || target.machineIndex === state.machineIndex)
      && (target.focus !== "remote-kernel" || target.remoteKernelIndex === state.remoteKernelIndex)
      && (target.focus !== "terminal" || target.terminalIndex === state.terminalIndex)
    )),
  )
  const next = order[modulo(currentIndex + delta, order.length)] ?? order[0]
  if (!next) {
    return state
  }

  return {
    ...state,
    focus: next.focus,
    sessionIndex: next.focus === "session" ? next.sessionIndex : state.sessionIndex,
    machineIndex: next.focus === "machine" ? next.machineIndex : state.machineIndex,
    remoteKernelIndex: next.focus === "remote-kernel" ? next.remoteKernelIndex : state.remoteKernelIndex,
    terminalIndex: next.focus === "terminal" ? next.terminalIndex : state.terminalIndex,
  }
}

export function cycleWaitingRoomValue(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  delta: number,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
  remote: WaitingRoomRemoteState = {},
) {
  if (state.focus === "model") {
    const options = catalogModelOptions(catalog, state.providerId)
    if (options.length === 0) {
      return state
    }
    const index = Math.max(0, options.findIndex((option) => option.id === state.modelId))
    const next = options[modulo(index + delta, options.length)]!
    return normalizeWaitingRoomState(
      {
        ...state,
        modelId: next.id,
      },
      sessions,
      catalog,
      themeRegistry,
      remote,
    )
  }
  if (state.focus === "provider") {
    const index = Math.max(0, BACKEND_PROVIDER_IDS.indexOf(state.providerId))
    return normalizeWaitingRoomState(
      {
        ...state,
        providerId: BACKEND_PROVIDER_IDS[modulo(index + delta, BACKEND_PROVIDER_IDS.length)]!,
      },
      sessions,
      catalog,
      themeRegistry,
      remote,
    )
  }
  if (state.focus === "effort") {
    const efforts = waitingRoomEfforts(waitingRoomModel(state, catalog))
    const index = Math.max(0, efforts.indexOf(state.effort))
    return {
      ...state,
      effort: efforts[modulo(index + delta, efforts.length)] ?? "",
    }
  }
  if (state.focus === "theme") {
    const options = themeOptions(themeRegistry)
    const ids = options.map((option) => option.id)
    const index = Math.max(0, ids.indexOf(normalizeThemeName(state.themeId, themeRegistry)))
    return {
      ...state,
      themeId: ids[modulo(index + delta, ids.length)] ?? normalizeThemeName(state.themeId, themeRegistry),
    }
  }
  if (state.focus === "worktree") {
    return {
      ...state,
      worktreeSelectionId: cycleWaitingRoomWorktreeSelectionId(state.worktreeSelectionId, delta),
    }
  }
  if (state.focus === "slice") {
    const options = waitingRoomSliceOptions({ slices: waitingRoomSlices(remote), selectionId: state.sliceSelectionId })
    const index = Math.max(0, options.findIndex((option) => option.id === state.sliceSelectionId))
    return {
      ...state,
      sliceSelectionId: options[modulo(index + delta, options.length)]?.id ?? "none",
    }
  }
  return state
}

export function waitingRoomChoice(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  remote: WaitingRoomRemoteState = {},
) {
  const visibleSessions = waitingRoomSessions(sessions)
  const model = waitingRoomModel(state, catalog)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const terminals = waitingRoomTerminals(remote)
  const slices = waitingRoomSlices(remote)
  return {
    session: visibleSessions[state.sessionIndex] ?? null,
    remoteMachine: remoteMachines[state.machineIndex] ?? null,
    remoteKernel: remoteKernels[state.remoteKernelIndex] ?? null,
    terminal: terminals[state.terminalIndex] ?? null,
    slice: waitingRoomSelectedSlice(state.sliceSelectionId, slices),
    sliceRef: selectedWaitingRoomSliceRef(state.sliceSelectionId, slices),
    providerId: state.providerId,
    model,
    effort: state.effort,
  }
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
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const sessionWindow = { start: 0, count: previewSessions.length }
  const sessionScrollbar = renderWaitingRoomScrollbar(sessionWindow.count, previewSessions.length, sessionWindow.start)
  const windowSessions = previewSessions
  const allSessionTitles = visibleSessions.map(formatWaitingRoomSessionTitle)
  const terminals = waitingRoomTerminals(remote)
  const terminalTitles = terminals.map(formatWaitingRoomTerminalTitle)
  const selectedWorktreeLabel = describeWaitingRoomWorktreeSelection(
    state.worktreeSelectionId,
    targets?.worktreePath,
  )
  const selectedSliceLabel = formatWaitingRoomSliceSelection(state.sliceSelectionId, waitingRoomSlices(remote))
  const statusWidth = Math.max(
    WAITING_ROOM_STATUS_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionStatus(session).length),
  )
  const lastUsedWidth = Math.max(
    "Last used".length,
    WAITING_ROOM_TIMESTAMP_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionTimestamp(session.last_used_at_ms ?? null).length),
  )
  const createdAtWidth = Math.max(
    "Created at".length,
    WAITING_ROOM_TIMESTAMP_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionTimestamp(session.created_at_ms ?? null).length),
  )
  const titleWidth = Math.max(
    WAITING_ROOM_ROW_TITLE_MIN_WIDTH,
    ...allSessionTitles.map((title) => Math.max(0, title.length)),
    ...terminalTitles.map((title) => Math.max(0, title.length)),
    "Add New Terminal".length,
  )
  const rows: WaitingRoomRow[] = [
    {
      id: "new",
      title: "Start New Session",
      value: "Press Enter",
      titleWidth,
      indent: 0,
      focused: state.focus === "new",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "provider",
      title: "Provider",
      value: formatBackendProviderLabel(choice.providerId),
      titleWidth,
      indent: 1,
      focused: state.focus === "provider",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "model",
      title: "Model",
      value: choice.model ? formatWaitingRoomModelLabel(choice.model, modelOptions) : "No models available",
      titleWidth,
      indent: 1,
      focused: state.focus === "model",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "effort",
      title: "Variant",
      value: choice.effort ? formatTitleCase(choice.effort) : "Default",
      titleWidth,
      indent: 1,
      focused: state.focus === "effort",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "workspace",
      title: "Workspace",
      value: targets?.workspacePath ?? (inventoryLoading ? loadingText : "Set workspace path"),
      titleWidth,
      indent: 1,
      focused: state.focus === "workspace",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "worktree",
      title: "Worktree",
      value: targets?.worktreePath ? selectedWorktreeLabel : inventoryLoading ? loadingText : selectedWorktreeLabel,
      titleWidth,
      indent: 1,
      focused: state.focus === "worktree",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "slice",
      title: "Slice",
      value: inventoryLoading ? loadingText : selectedSliceLabel,
      titleWidth,
      indent: 1,
      focused: state.focus === "slice",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "join-header",
      title: "Join Existing Session",
      value: inventoryLoading && visibleSessions.length === 0 ? loadingText : visibleSessions.length > 0 ? "Press Enter" : "",
      titleWidth,
      indent: 0,
      focused: state.focus === "join-sessions",
      selectable: true,
      scrollbar: "",
    },
  ]

  if (visibleSessions.length === 0 && inventoryLoading) {
    rows.push({
      id: "sessions-loading",
      title: "Sessions",
      value: loadingText,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
  } else if (visibleSessions.length === 0) {
    rows.push({
      id: "no-sessions",
      title: "No sessions available",
      value: "",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
  } else {
    rows.push({
      id: "session-header",
      title: "Session",
      value: "",
      titleWidth,
      columns: [
        formatWaitingRoomColumnHeader("Status", statusWidth),
        formatWaitingRoomColumnHeader("Last used", lastUsedWidth),
        formatWaitingRoomColumnHeader("Created at", createdAtWidth),
      ],
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })

    for (const [offset, session] of windowSessions.entries()) {
      const sessionIndex = visibleSessions.findIndex((candidate) => candidate.id === session.id)
      rows.push({
        id: `session:${session.id}`,
        title: formatWaitingRoomSessionTitle(session),
        value: formatWaitingRoomSessionStatus(session),
        titleWidth,
        columns: [
          formatWaitingRoomColumn(formatWaitingRoomSessionStatus(session), statusWidth),
          formatWaitingRoomColumn(formatSessionTimestamp(session.last_used_at_ms ?? null), lastUsedWidth),
          formatWaitingRoomColumn(formatSessionTimestamp(session.created_at_ms ?? null), createdAtWidth),
        ],
        indent: 1,
        focused: state.focus === "session" && state.sessionIndex === sessionIndex,
        selectable: true,
        scrollbar: sessionScrollbar[offset] ?? "",
      })
    }
  }

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

function waitingRoomSlices(remote: WaitingRoomRemoteState = {}) {
  return (remote.slices ?? [])
    .slice()
    .sort((left, right) => formatWaitingRoomSliceLabel(left).localeCompare(formatWaitingRoomSliceLabel(right)))
}

function waitingRoomSliceOptions(options: { slices: SliceRecord[]; selectionId?: string | null | undefined }) {
  return [
    { id: "none", slice: null },
    ...options.slices.map((slice) => ({ id: slice.id, slice })),
  ]
}

function normalizeWaitingRoomSliceSelectionId(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const normalized = selectionId?.trim() || "none"
  if (normalized === "none") {
    return "none"
  }
  return waitingRoomSliceOptions({ slices, selectionId: normalized }).some((option) => option.id === normalized)
    ? normalized
    : "none"
}

function waitingRoomSelectedSlice(selectionId: string | null | undefined, slices: SliceRecord[]) {
  if (selectionId === "none") {
    return null
  }
  return slices.find((slice) => slice.id === selectionId || slice.name === selectionId) ?? null
}

function selectedWaitingRoomSliceRef(selectionId: string | null | undefined, slices: SliceRecord[]) {
  return waitingRoomSelectedSlice(selectionId, slices)?.id ?? null
}

function formatWaitingRoomSliceSelection(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const slice = waitingRoomSelectedSlice(selectionId, slices)
  return slice ? formatWaitingRoomSliceLabel(slice) : "None"
}

function formatWaitingRoomSliceLabel(slice: SliceRecord) {
  return slice.name || slice.id
}

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}

export function waitingRoomMenuMinWidth(sessions: SessionListEntry[]) {
  const visibleSessions = waitingRoomSessions(sessions)
  const allSessionTitles = visibleSessions.map(formatWaitingRoomSessionTitle)

  const statusWidth = Math.max(
    WAITING_ROOM_STATUS_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionStatus(session).length),
  )
  const lastUsedWidth = Math.max(
    "Last used".length,
    WAITING_ROOM_TIMESTAMP_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionTimestamp(session.last_used_at_ms ?? null).length),
  )
  const createdAtWidth = Math.max(
    "Created at".length,
    WAITING_ROOM_TIMESTAMP_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionTimestamp(session.created_at_ms ?? null).length),
  )
  const titleWidth = Math.max(
    WAITING_ROOM_ROW_TITLE_MIN_WIDTH,
    ...allSessionTitles.map((title) => Math.max(0, title.length)),
  )

  const titleWidthSpace = Math.max(0, titleWidth - 1)
  const rowColumns = [
    formatWaitingRoomColumnHeader("Status", statusWidth),
    formatWaitingRoomColumnHeader("Last used", lastUsedWidth),
    formatWaitingRoomColumnHeader("Created at", createdAtWidth),
  ]
  const row = ` ${"  ".repeat(1)}${"Session".padEnd(titleWidthSpace, " ")} ${rowColumns.join("  ")}${" ".repeat(WAITING_ROOM_MENU_TRAILING_PADDING)}`
  return row.length
}

export function waitingRoomMenuTrailingPadding() {
  return WAITING_ROOM_MENU_TRAILING_PADDING
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

function waitingRoomSessions(sessions: SessionListEntry[]) {
  return sessions
    .filter((session) => session.status !== "Ended")
    .slice()
    .sort((left, right) => sessionSortTime(right) - sessionSortTime(left))
}

export function waitingRoomPreviewSessions(sessions: SessionListEntry[]) {
  return waitingRoomSessions(sessions).slice(0, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
}

function sessionSortTime(session: SessionListEntry) {
  return session.last_used_at_ms ?? session.created_at_ms ?? 0
}

function waitingRoomFocusTargets(sessions: SessionListEntry[], remote: WaitingRoomRemoteState = {}) {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const terminals = waitingRoomTerminals(remote)
  const slices = waitingRoomSlices(remote)
  return [
    { focus: "new" as const, sessionIndex: 0 },
    { focus: "provider" as const, sessionIndex: 0 },
    { focus: "model" as const, sessionIndex: 0 },
    { focus: "effort" as const, sessionIndex: 0 },
    { focus: "workspace" as const, sessionIndex: 0 },
    { focus: "worktree" as const, sessionIndex: 0 },
    ...(slices.length > 0 ? [{ focus: "slice" as const, sessionIndex: 0 }] : []),
    ...(visibleSessions.length > 0 ? [{ focus: "join-sessions" as const, sessionIndex: 0 }] : []),
    ...previewSessions.map((session) => ({
      focus: "session" as const,
      sessionIndex: Math.max(0, visibleSessions.findIndex((candidate) => candidate.id === session.id)),
    })),
    { focus: "relay" as const, sessionIndex: 0 },
    ...remoteMachines.map((_, machineIndex) => ({
      focus: "machine" as const,
      sessionIndex: 0,
      machineIndex,
    })),
    ...remoteKernels.map((_, remoteKernelIndex) => ({
      focus: "remote-kernel" as const,
      sessionIndex: 0,
      remoteKernelIndex,
    })),
    ...terminals.map((_, terminalIndex) => ({
      focus: "terminal" as const,
      sessionIndex: 0,
      terminalIndex,
    })),
    { focus: "add-terminal" as const, sessionIndex: 0 },
    { focus: "theme" as const, sessionIndex: 0 },
  ].map((target) => ({
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    ...target,
  }))
}

function waitingRoomSessionWindow(state: WaitingRoomState, sessions: SessionListEntry[]) {
  const count = Math.min(MAX_VISIBLE_WAITING_ROOM_SESSIONS, sessions.length)
  if (count === 0) {
    return { start: 0, count: 0 }
  }
  const maxStart = Math.max(0, sessions.length - count)
  const start = Math.min(Math.max(0, state.sessionIndex - count + 1), maxStart)
  return { start, count }
}

function renderWaitingRoomScrollbar(visibleCount: number, totalCount: number, start: number) {
  if (visibleCount === 0 || totalCount <= visibleCount) {
    return []
  }
  const thumbSize = Math.max(1, Math.round((visibleCount * visibleCount) / totalCount))
  const maxThumbOffset = Math.max(0, visibleCount - thumbSize)
  const thumbOffset = totalCount === visibleCount
    ? 0
    : Math.round((start * maxThumbOffset) / Math.max(1, totalCount - visibleCount))
  return Array.from({ length: visibleCount }, (_, index) => (
    index >= thumbOffset && index < thumbOffset + thumbSize ? "#" : "|"
  ))
}

function formatTitleCase(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function formatSessionStatus(value: string) {
  return formatTitleCase(value.toLowerCase())
}

function formatWaitingRoomSessionStatus(session: SessionListEntry) {
  return sessionHasActiveWork(session) ? "Working" : formatSessionStatus(session.status)
}

function sessionHasActiveWork(session: SessionListEntry) {
  const activity = session.activity
  if (!activity) {
    return false
  }
  return activity.working_agent_count > 0
    || activity.active_prompt_count > 0
    || activity.queued_prompt_count > 0
}

function formatSessionTimestamp(value: number | null) {
  if (value === null) {
    return "—"
  }

  const date = new Date(value)
  const year = date.getUTCFullYear()
  const month = String(date.getUTCMonth() + 1).padStart(2, "0")
  const day = String(date.getUTCDate()).padStart(2, "0")
  const hours = String(date.getUTCHours()).padStart(2, "0")
  const minutes = String(date.getUTCMinutes()).padStart(2, "0")
  return `${year}-${month}-${day} ${hours}:${minutes} UTC`
}

function formatWaitingRoomColumnHeader(label: string, width: number) {
  return label.padEnd(width, " ")
}

function formatWaitingRoomColumn(value: string, width: number) {
  return value.padEnd(width, " ")
}

function normalizeBackendProvider(value: string): BackendProviderId {
  return normalizeBackendProviderId(value)
}

function formatBackendProviderLabel(providerId: BackendProviderId) {
  return backendProviderLabel(providerId)
}

function formatWaitingRoomModelLabel(
  model: CatalogModelOption,
  options: CatalogModelOption[],
) {
  const providerCount = new Set(options.map((option) => option.providerId)).size
  return providerCount <= 1 ? model.label : `${model.providerName} ${model.label}`
}
