import { ARROBA_ASCII_ART, type SessionListEntry } from "./sessions.js"
import {
  catalogModelOptions,
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

export const MAX_VISIBLE_WAITING_ROOM_SESSIONS = 10
const WAITING_ROOM_ROW_TITLE_MIN_WIDTH = 24
const WAITING_ROOM_STATUS_MIN_WIDTH = "Status".length
const WAITING_ROOM_TIMESTAMP_MIN_WIDTH = "0000-00-00 00:00 UTC".length
const WAITING_ROOM_MENU_TRAILING_PADDING = 2

function formatWaitingRoomSessionTitle(session: SessionListEntry) {
  if (!session.alias) {
    return session.id
  }
  return `${session.id} (${session.alias})`
}

export type WaitingRoomFocus = "new" | "provider" | "model" | "effort" | "theme" | "session" | "relay"

export type WaitingRoomKeyState = {
  up: boolean
  down: boolean
  left: boolean
  right: boolean
}

export type WaitingRoomState = {
  focus: WaitingRoomFocus
  sessionIndex: number
  providerId: BackendProviderId
  modelId: string
  effort: string
  themeId: ThemeName
  introStep: number
  keyState: WaitingRoomKeyState
}

export type WaitingRoomRemoteState = {
  relay?: {
    configured: boolean
    connected: boolean
    relay_url?: string | null
  } | null
  machines?: Array<{
    machine_id: string
    machine_alias?: string | null
    registry_alias?: string | null
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
    kernel_count: number
    available_providers?: string[]
    pending?: boolean
  }>
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
) {
  const visibleSessions = waitingRoomSessions(sessions)
  const providerId = normalizeBackendProvider(state.providerId)
  const selected = selectConfiguredModel(catalog, state.modelId, providerId)
  const efforts = waitingRoomEfforts(selected)
  return {
    ...state,
    focus: visibleSessions.length === 0 && state.focus === "session" ? "new" : state.focus,
    providerId,
    sessionIndex: visibleSessions.length === 0 ? 0 : modulo(state.sessionIndex, visibleSessions.length),
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

export function moveWaitingRoomFocus(state: WaitingRoomState, sessions: SessionListEntry[], delta: number) {
  const order = waitingRoomFocusTargets(sessions)
  const currentIndex = Math.max(
    0,
    order.findIndex((target) => (
      target.focus === state.focus
      && (target.focus !== "session" || target.sessionIndex === state.sessionIndex)
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
  }
}

export function cycleWaitingRoomValue(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  delta: number,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
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
    )
  }
  if (state.focus === "provider") {
    const providers: BackendProviderId[] = ["opencode", "codex"]
    const index = Math.max(0, providers.indexOf(state.providerId))
    return normalizeWaitingRoomState(
      {
        ...state,
        providerId: providers[modulo(index + delta, providers.length)]!,
      },
      sessions,
      catalog,
      themeRegistry,
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
  return state
}

export function waitingRoomChoice(state: WaitingRoomState, sessions: SessionListEntry[], catalog: ProviderCatalog) {
  const visibleSessions = waitingRoomSessions(sessions)
  const model = waitingRoomModel(state, catalog)
  return {
    session: visibleSessions[state.sessionIndex] ?? null,
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
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
) {
  const choice = waitingRoomChoice(state, sessions, catalog)
  const modelOptions = catalogModelOptions(catalog, state.providerId)
  const visibleSessions = waitingRoomSessions(sessions)
  const sessionWindow = waitingRoomSessionWindow(state, visibleSessions)
  const sessionScrollbar = renderWaitingRoomScrollbar(sessionWindow.count, visibleSessions.length, sessionWindow.start)
  const windowSessions = visibleSessions.slice(sessionWindow.start, sessionWindow.start + sessionWindow.count)
  const allSessionTitles = visibleSessions.map(formatWaitingRoomSessionTitle)
  const statusWidth = Math.max(
    WAITING_ROOM_STATUS_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionStatus(session.status).length),
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
      id: "join-header",
      title: "Join Existing Session",
      value: "",
      titleWidth,
      indent: 0,
      focused: false,
      selectable: true,
      scrollbar: "",
    },
  ]

  if (visibleSessions.length === 0) {
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
      const sessionIndex = sessionWindow.start + offset
      rows.push({
        id: `session:${session.id}`,
        title: formatWaitingRoomSessionTitle(session),
        value: formatSessionStatus(session.status),
        titleWidth,
        columns: [
          formatWaitingRoomColumn(formatSessionStatus(session.status), statusWidth),
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


function waitingRoomRemoteRows(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
  titleWidth: number,
): WaitingRoomRow[] {
  const relay = remote.relay ?? null
  const machines = remote.machines ?? []
  const pendingCount = machines.filter((machine) => machine.pending).length
  const onlineMachines = machines.filter((machine) => machine.online !== false)
  const relayStatus = !relay || !relay.configured
    ? "not configured"
    : relay.connected
      ? `connected ${relay.relay_url ?? ""}`.trim()
      : `connecting ${relay.relay_url ?? ""}`.trim()
  const rows: WaitingRoomRow[] = [
    {
      id: "relay-header",
      title: "Relay",
      value: relayStatus,
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
    {
      id: "relay-configure",
      title: "Configure Relay",
      value: "/relay use <ws-url> <token>",
      titleWidth,
      indent: 1,
      focused: state.focus === "relay",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "machines-header",
      title: "Machines",
      value: `${onlineMachines.length} online${pendingCount > 0 ? ` (${pendingCount} pending)` : ""}`,
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
  ]

  if (!relay?.configured) {
    rows.push({
      id: "machines-unavailable",
      title: "Remote Machines",
      value: "unavailable until relay is configured",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    return rows
  }

  if (machines.length === 0) {
    rows.push({
      id: "machines-none",
      title: "Remote Machines",
      value: relay.connected ? "none online" : "waiting for relay connection",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    return rows
  }

  for (const machine of machines.slice(0, 4)) {
    const label = machine.display_name ?? machine.registry_alias ?? machine.machine_alias ?? machine.machine_id
    const providers = (machine.available_providers ?? []).join(",") || "no providers"
    const status = machine.online === false ? "offline" : machine.pending ? "pending" : "approved"
    rows.push({
      id: `machine:${machine.machine_id}`,
      title: `${label}${status !== "approved" ? ` (${status})` : ""}`,
      value: `${machine.kernel_count} kernel${machine.kernel_count === 1 ? "" : "s"} ${providers}`,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
  }
  if (machines.length > 4) {
    rows.push({
      id: "machines-more",
      title: "More Machines",
      value: `/machine list (${machines.length - 4} more)`,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
  }
  return rows
}

export function waitingRoomMenuMinWidth(sessions: SessionListEntry[]) {
  const visibleSessions = waitingRoomSessions(sessions)
  const allSessionTitles = visibleSessions.map(formatWaitingRoomSessionTitle)

  const statusWidth = Math.max(
    WAITING_ROOM_STATUS_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionStatus(session.status).length),
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
  return sessions.filter((session) => session.status !== "Ended")
}

function waitingRoomFocusTargets(sessions: SessionListEntry[]) {
  const visibleSessions = waitingRoomSessions(sessions)
  return [
    { focus: "new" as const, sessionIndex: 0 },
    { focus: "provider" as const, sessionIndex: 0 },
    { focus: "model" as const, sessionIndex: 0 },
    { focus: "effort" as const, sessionIndex: 0 },
    ...visibleSessions.map((_, sessionIndex) => ({ focus: "session" as const, sessionIndex })),
    { focus: "relay" as const, sessionIndex: 0 },
    { focus: "theme" as const, sessionIndex: 0 },
  ]
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
  return value === "codex" ? "codex" : "opencode"
}

function formatBackendProviderLabel(providerId: BackendProviderId) {
  return providerId === "codex" ? "Codex" : "OpenCode"
}

function formatWaitingRoomModelLabel(
  model: CatalogModelOption,
  options: CatalogModelOption[],
) {
  const providerCount = new Set(options.map((option) => option.providerId)).size
  return providerCount <= 1 ? model.label : `${model.providerName} ${model.label}`
}
