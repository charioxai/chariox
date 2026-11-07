// The list card's rows: the hierarchy of the room agent's sub-agents, one plain state word each.
import type { SubAgentRow } from "@chariox/kernel-client/sub-agent-list"
import type { EnvironmentDialogRow } from "./project-environment-projection.js"

/** `lines` are the visible rows in order; the state word sits at the right edge, a long name clips before it. */
export function subAgentsCardRows(lines: readonly SubAgentRow[], selected: number, collapsed: ReadonlySet<string>, width: number): EnvironmentDialogRow[] {
  return lines.map((row, index) => {
    const twisty = row.children.length === 0 ? "  " : collapsed.has(row.id) ? "▸ " : "▾ "
    const left = `${"  ".repeat(row.depth)}${twisty}${row.name}`
    const room = Math.max(1, width - row.word.length - 1)
    const text = `${left.length > room ? `${left.slice(0, room - 1)}…` : left.padEnd(room)} ${row.word}`
    return { text, tone: row.dimmed ? "muted" : row.word === "needs you" ? "warning" : "text", focused: index === selected, bold: false }
  })
}
