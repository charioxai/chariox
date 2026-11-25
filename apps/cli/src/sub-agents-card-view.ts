// The list card's rows: the hierarchy of the room agent's sub-agents, one status each: the wait reason with its age, or the state in a word.
import type { SubAgentRow } from "@chariox/kernel-client/sub-agent-list"
import type { EnvironmentDialogRow } from "./project-environment-projection.js"

/** `lines` are the visible rows in order; the status sits at the right edge, a long name clips before it. */
export function subAgentsCardRows(lines: readonly SubAgentRow[], selected: number, collapsed: ReadonlySet<string>, width: number): EnvironmentDialogRow[] {
  return lines.map((row, index) => {
    const twisty = row.children.length === 0 ? "  " : collapsed.has(row.id) ? "▸ " : "▾ "
    const origin = row.startedBy ? ` · Started by ${row.startedBy}` : ""
    const location = row.roomId ? ` · ${row.roomName ?? "another Room"}` : ""
    const left = `${"  ".repeat(row.depth)}${twisty}${row.name}${origin}${location}`
    // A long reason leaves the name its first 13 columns.
    const limit = Math.max(8, width - 14)
    const status = row.status.length > limit ? `${row.status.slice(0, limit - 1)}…` : row.status
    const room = Math.max(1, width - status.length - 1)
    const text = `${left.length > room ? `${left.slice(0, room - 1)}…` : left.padEnd(room)} ${status}`
    return { text, tone: row.dimmed ? "muted" : row.word === "needs you" ? "warning" : "text", focused: index === selected, bold: false }
  })
}
