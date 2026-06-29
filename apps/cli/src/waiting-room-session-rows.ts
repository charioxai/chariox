import {
  waitingRoomSessionActivityHasUnreadIdleOutput,
  waitingRoomSessionActivityHasWork,
} from "@arroba/kernel-client/waiting-room-activity"

import {
  formatSessionActivityNextAction,
  formatSessionHomeLabel,
  formatSessionLiveSyncLabel,
  type SessionListEntry,
} from "./sessions.js"
import type { WaitingRoomRow, WaitingRoomState } from "./waiting-room-types.js"

export const MAX_VISIBLE_WAITING_ROOM_SESSIONS = 2

const WAITING_ROOM_ROW_TITLE_MIN_WIDTH = 24
const WAITING_ROOM_STATUS_MIN_WIDTH = "Status".length
const WAITING_ROOM_HOME_MIN_WIDTH = "Home".length
const WAITING_ROOM_SYNC_MIN_WIDTH = "Sync".length
const WAITING_ROOM_WORK_MIN_WIDTH = "Work".length
const WAITING_ROOM_NEXT_MIN_WIDTH = "Next".length
const WAITING_ROOM_TIMESTAMP_MIN_WIDTH = "0000-00-00 00:00 UTC".length
const WAITING_ROOM_MENU_TRAILING_PADDING = 2

export function waitingRoomSessionRows(
  state: Pick<WaitingRoomState, "focus" | "sessionIndex">,
  sessions: SessionListEntry[],
  options: {
    inventoryLoading: boolean
    loadingText: string
    titleWidth: number
  },
): WaitingRoomRow[] {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const sessionWindow = { start: 0, count: previewSessions.length }
  const sessionScrollbar = renderWaitingRoomScrollbar(sessionWindow.count, previewSessions.length, sessionWindow.start)
  const statusWidth = Math.max(
    WAITING_ROOM_STATUS_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionStatus(session).length),
  )
  const homeWidth = Math.max(
    WAITING_ROOM_HOME_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionHomeLabel(session).length),
  )
  const syncWidth = Math.max(
    WAITING_ROOM_SYNC_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionLiveSyncLabel(session).length),
  )
  const workWidth = Math.max(
    WAITING_ROOM_WORK_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionWork(session).length),
  )
  const nextWidth = Math.max(
    WAITING_ROOM_NEXT_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionNext(session).length),
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

  if (visibleSessions.length === 0 && options.inventoryLoading) {
    return [{
      id: "sessions-loading",
      title: "Sessions",
      value: options.loadingText,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    }]
  }
  if (visibleSessions.length === 0) {
    return [{
      id: "no-sessions",
      title: "No sessions available",
      value: "",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    }]
  }

  const rows: WaitingRoomRow[] = [{
    id: "session-header",
    title: "Session",
    value: "",
    titleWidth: options.titleWidth,
    columns: [
      formatWaitingRoomColumnHeader("Status", statusWidth),
      formatWaitingRoomColumnHeader("Home", homeWidth),
      formatWaitingRoomColumnHeader("Sync", syncWidth),
      formatWaitingRoomColumnHeader("Work", workWidth),
      formatWaitingRoomColumnHeader("Next", nextWidth),
      formatWaitingRoomColumnHeader("Last used", lastUsedWidth),
      formatWaitingRoomColumnHeader("Created at", createdAtWidth),
    ],
    indent: 1,
    focused: false,
    selectable: false,
    scrollbar: "",
  }]

  for (const [offset, session] of previewSessions.entries()) {
    const sessionIndex = visibleSessions.findIndex((candidate) => candidate.id === session.id)
    rows.push({
      id: `session:${session.id}`,
      title: formatWaitingRoomSessionTitle(session),
      value: formatWaitingRoomSessionStatus(session),
      titleWidth: options.titleWidth,
      columns: [
        formatWaitingRoomColumn(formatWaitingRoomSessionStatus(session), statusWidth),
        formatWaitingRoomColumn(formatSessionHomeLabel(session), homeWidth),
        formatWaitingRoomColumn(formatSessionLiveSyncLabel(session), syncWidth),
        formatWaitingRoomColumn(formatWaitingRoomSessionWork(session), workWidth),
        formatWaitingRoomColumn(formatWaitingRoomSessionNext(session), nextWidth),
        formatWaitingRoomColumn(formatSessionTimestamp(session.last_used_at_ms ?? null), lastUsedWidth),
        formatWaitingRoomColumn(formatSessionTimestamp(session.created_at_ms ?? null), createdAtWidth),
      ],
      indent: 1,
      focused: state.focus === "session" && state.sessionIndex === sessionIndex,
      selectable: true,
      scrollbar: sessionScrollbar[offset] ?? "",
    })
  }
  return rows
}

export function waitingRoomSessionTitleWidth(sessions: SessionListEntry[]) {
  const visibleSessions = waitingRoomSessions(sessions)
  return Math.max(
    WAITING_ROOM_ROW_TITLE_MIN_WIDTH,
    ...visibleSessions.map((session) => Math.max(0, formatWaitingRoomSessionTitle(session).length)),
  )
}

export function waitingRoomMenuMinWidth(sessions: SessionListEntry[]) {
  const visibleSessions = waitingRoomSessions(sessions)

  const statusWidth = Math.max(
    WAITING_ROOM_STATUS_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionStatus(session).length),
  )
  const homeWidth = Math.max(
    WAITING_ROOM_HOME_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionHomeLabel(session).length),
  )
  const syncWidth = Math.max(
    WAITING_ROOM_SYNC_MIN_WIDTH,
    ...visibleSessions.map((session) => formatSessionLiveSyncLabel(session).length),
  )
  const workWidth = Math.max(
    WAITING_ROOM_WORK_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionWork(session).length),
  )
  const nextWidth = Math.max(
    WAITING_ROOM_NEXT_MIN_WIDTH,
    ...visibleSessions.map((session) => formatWaitingRoomSessionNext(session).length),
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
  const titleWidth = waitingRoomSessionTitleWidth(sessions)

  const titleWidthSpace = Math.max(0, titleWidth - 1)
  const rowColumns = [
    formatWaitingRoomColumnHeader("Status", statusWidth),
    formatWaitingRoomColumnHeader("Home", homeWidth),
    formatWaitingRoomColumnHeader("Sync", syncWidth),
    formatWaitingRoomColumnHeader("Work", workWidth),
    formatWaitingRoomColumnHeader("Next", nextWidth),
    formatWaitingRoomColumnHeader("Last used", lastUsedWidth),
    formatWaitingRoomColumnHeader("Created at", createdAtWidth),
  ]
  const row = ` ${"  ".repeat(1)}${"Session".padEnd(titleWidthSpace, " ")} ${rowColumns.join("  ")}${" ".repeat(WAITING_ROOM_MENU_TRAILING_PADDING)}`
  return row.length
}

export function waitingRoomMenuTrailingPadding() {
  return WAITING_ROOM_MENU_TRAILING_PADDING
}

export function waitingRoomSessions(sessions: SessionListEntry[]) {
  return sessions
    .filter((session) => session.status !== "Ended")
    .slice()
    .sort((left, right) => sessionLastActiveMs(right) - sessionLastActiveMs(left))
}

export function waitingRoomPreviewSessions(sessions: SessionListEntry[]) {
  return waitingRoomSessions(sessions).slice(0, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
}

export function sessionLastActiveMs(session: SessionListEntry) {
  return numberOrZero(session.last_prompt_sent_at_ms)
    || numberOrZero(session.last_activity_at_ms)
    || numberOrZero(session.last_used_at_ms)
    || numberOrZero(session.created_at_ms)
}

function numberOrZero(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0
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

function formatWaitingRoomSessionTitle(session: SessionListEntry) {
  const label = session.alias ? `${session.id} (${session.alias})` : session.id
  return sessionHasActiveWork(session) ? `* ${label}` : label
}

function formatWaitingRoomSessionStatus(session: SessionListEntry) {
  if (sessionHasActiveWork(session)) {
    return "Working"
  }
  if (sessionHasUnreadIdleOutput(session)) {
    return "Done"
  }
  return formatSessionStatus(session.status)
}

function formatWaitingRoomSessionNext(session: SessionListEntry) {
  return formatSessionActivityNextAction(session) ?? "-"
}

function formatWaitingRoomSessionWork(session: SessionListEntry) {
  const activity = session.activity
  if (!activity) {
    return "-"
  }
  const parts = [
    activity.working_agent_count > 0 ? `${activity.working_agent_count} working` : "",
    activity.active_prompt_count > 0
      ? `${activity.active_prompt_count} active prompt${activity.active_prompt_count === 1 ? "" : "s"}`
      : "",
    activity.queued_prompt_count > 0
      ? `${activity.queued_prompt_count} queued prompt${activity.queued_prompt_count === 1 ? "" : "s"}`
      : "",
  ].filter(Boolean)
  return parts.length ? parts.join(", ") : "-"
}

function sessionHasActiveWork(session: SessionListEntry) {
  return waitingRoomSessionActivityHasWork(session.activity)
}

function sessionHasUnreadIdleOutput(session: SessionListEntry) {
  return waitingRoomSessionActivityHasUnreadIdleOutput(session.activity)
}

function formatSessionStatus(value: string) {
  return formatTitleCase(value.toLowerCase())
}

function formatTitleCase(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
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
