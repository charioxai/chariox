import { waitingRoomMenuTrailingPadding } from "./waiting-room-session-rows.js"
import type { WaitingRoomRow } from "./waiting-room-types.js"

export function formatWaitingRoomMenuRow(row: WaitingRoomRow, maxWidth = Number.POSITIVE_INFINITY) {
  const prefix = row.focused ? ">" : " "
  const indent = "  ".repeat(row.indent)
  const titleWidth = Math.max(24, row.titleWidth ?? 24)
  const value = row.columns ? ` ${row.columns.join("  ")}` : row.value ? ` ${row.value}` : ""
  const scrollbar = row.scrollbar ? `  ${row.scrollbar}` : ""
  const titleWidthSpace = Math.max(0, titleWidth - row.indent)
  const content = `${prefix} ${indent}${row.title.padEnd(titleWidthSpace, " ")}${value}${scrollbar}${" ".repeat(waitingRoomMenuTrailingPadding())}`
  return truncateLine(content, maxWidth)
}

function truncateLine(value: string, maxWidth: number) {
  const width = Math.floor(maxWidth)
  if (!Number.isFinite(width) || width <= 0 || value.length <= width) {
    return value
  }
  if (width <= 3) {
    return ".".repeat(width)
  }
  return `${value.slice(0, width - 3)}...`
}
