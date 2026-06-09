import type { ExternalProviderSessionRecord } from "./cli-types.js"
import type { WaitingRoomRemoteState, WaitingRoomRow, WaitingRoomState } from "./waiting-room-types.js"

const TITLE_MIN_WIDTH = 24
const PROVIDER_MIN_WIDTH = "Provider".length
const MODE_MIN_WIDTH = "Mode".length
const MODIFIED_MIN_WIDTH = "0000-00-00 00:00 UTC".length

export function waitingRoomExternalProviderSessionRows(
  state: Pick<WaitingRoomState, "focus" | "externalSessionIndex">,
  remote: WaitingRoomRemoteState = {},
  options: {
    inventoryLoading: boolean
    loadingText: string
    titleWidth: number
  },
): WaitingRoomRow[] {
  const sessions = waitingRoomExternalProviderSessions(remote)
  if (sessions.length === 0 && options.inventoryLoading) {
    return [{
      id: "external-provider-sessions-loading",
      title: "External sessions",
      value: options.loadingText,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    }]
  }
  if (sessions.length === 0) {
    return []
  }

  const providerWidth = Math.max(PROVIDER_MIN_WIDTH, ...sessions.map((session) => session.provider.length))
  const modeWidth = Math.max(MODE_MIN_WIDTH, ...sessions.map((session) => formatMode(session).length))
  const modifiedWidth = Math.max(
    "Modified".length,
    MODIFIED_MIN_WIDTH,
    ...sessions.map((session) => formatTimestamp(session.last_modified_at_ms).length),
  )
  const rows: WaitingRoomRow[] = [{
    id: "external-provider-session-header",
    title: "External session",
    value: "",
    titleWidth: options.titleWidth,
    columns: [
      columnHeader("Provider", providerWidth),
      columnHeader("Mode", modeWidth),
      columnHeader("Modified", modifiedWidth),
    ],
    indent: 1,
    focused: false,
    selectable: false,
    scrollbar: "",
  }]
  for (const [index, session] of sessions.entries()) {
    rows.push({
      id: `external-session:${session.external_session_id}`,
      title: formatTitle(session),
      value: session.provider,
      titleWidth: options.titleWidth,
      columns: [
        column(session.provider, providerWidth),
        column(formatMode(session), modeWidth),
        column(formatTimestamp(session.last_modified_at_ms), modifiedWidth),
      ],
      indent: 1,
      focused: state.focus === "external-session" && state.externalSessionIndex === index,
      selectable: true,
      scrollbar: "",
    })
  }
  if (remote.externalProviderSessionsHasMore) {
    rows.push({
      id: "external-provider-session-more",
      title: "Load older external sessions",
      value: "",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "external-session-more",
      selectable: true,
      scrollbar: "",
    })
  }
  return rows
}

export function waitingRoomExternalProviderSessionTitleWidth(remote: WaitingRoomRemoteState = {}) {
  return Math.max(
    TITLE_MIN_WIDTH,
    ...waitingRoomExternalProviderSessions(remote).map((session) => formatTitle(session).length),
  )
}

export function waitingRoomExternalProviderSessions(remote: WaitingRoomRemoteState = {}) {
  return (remote.externalProviderSessions ?? []).slice()
}

function formatTitle(session: ExternalProviderSessionRecord) {
  return session.title || session.first_prompt_preview || session.provider_session_id
}

function formatMode(session: ExternalProviderSessionRecord) {
  if (session.already_imported) {
    return "Imported"
  }
  return (session.mode ?? "observed").replace(/_/g, " ")
}

function formatTimestamp(value: number | null | undefined) {
  if (!value) {
    return "-"
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return "-"
  }
  return date.toISOString().replace("T", " ").slice(0, 16) + " UTC"
}

function column(value: string, width: number) {
  return value.padEnd(width, " ")
}

function columnHeader(value: string, width: number) {
  return value.padEnd(width, " ")
}
