// What the sub-agents stripe, list card and sub-view show, selected from the kernel's `AgentTree`: plain counts,
// one state word per agent, the hierarchy in creation order. Status only; a decision is never read here.
import type { AgentTree, AgentTreeNode } from "./kernel-types-runtime.js"

export type SubAgentWord = "running" | "waiting" | "needs you" | "done"

export interface SubAgentRow {
  readonly id: string
  readonly name: string
  readonly word: SubAgentWord
  /** Finished: shown dimmed until the parent's task ends and the kernel drops it. */
  readonly dimmed: boolean
  readonly depth: number
  readonly children: readonly SubAgentRow[]
}

/** One sub-view panel: the card row without its children. */
export type SubAgentPanel = Omit<SubAgentRow, "children">

export interface SubAgentStripe {
  readonly text: string
  readonly needsYou: boolean
}

const wordOf = (node: AgentTreeNode): Exclude<SubAgentWord, "needs you"> => {
  switch (node.state) {
    case "starting": case "running": return "running"
    case "waiting": case "unreachable": return "waiting"
    case "idle": case "ended": case "error": return "done"
  }
}

const byCreation = (a: AgentTreeNode, b: AgentTreeNode) =>
  a.created_at_ms - b.created_at_ms || (a.agent_id < b.agent_id ? -1 : a.agent_id > b.agent_id ? 1 : 0)

/** The hierarchy under the room agent, in creation order at every level. */
export function subAgentRows(tree: AgentTree): readonly SubAgentRow[] {
  const nodes = [...tree.nodes].sort(byCreation)
  const ids = new Set(nodes.map(node => node.agent_id))
  const seen = new Set<string>()
  const build = (node: AgentTreeNode, depth: number): SubAgentRow => {
    seen.add(node.agent_id)
    const base = wordOf(node)
    return {
      id: node.agent_id,
      name: node.alias ?? node.agent_id,
      word: base === "waiting" && node.needs_user === true ? "needs you" : base,
      dimmed: base === "done",
      depth,
      children: nodes.filter(child => child.parent_agent_id === node.agent_id && !seen.has(child.agent_id)).map(child => build(child, depth + 1)),
    }
  }
  const top = nodes.filter(node => node.parent_agent_id === tree.root || !ids.has(node.parent_agent_id ?? "")).map(node => build(node, 0))
  // A report with a loop of parents still lists every agent.
  const lost = nodes.flatMap(node => seen.has(node.agent_id) ? [] : [build(node, 0)])
  return [...top, ...lost]
}

/** The card's lines: depth first, without the descendants of a collapsed row. */
export function visibleSubAgentRows(rows: readonly SubAgentRow[], collapsed: ReadonlySet<string>): SubAgentRow[] {
  return rows.flatMap(row => [row, ...(collapsed.has(row.id) ? [] : visibleSubAgentRows(row.children, collapsed))])
}

/** The sub-view's panels: every sub-agent once, in card order. A held decision reads "waiting" there. */
export function subAgentPanels(tree: AgentTree): SubAgentPanel[] {
  const all = (rows: readonly SubAgentRow[]): SubAgentRow[] => rows.flatMap(row => [row, ...all(row.children)])
  return all(subAgentRows(tree)).map(({ children: _children, word, ...row }) => ({ ...row, word: word === "needs you" ? "waiting" : word }))
}

/** "6 agents running"; with waits, "1 agent needs you · 2 running · 1 waiting · 1 done". */
export function subAgentStripe(tree: AgentTree | null): SubAgentStripe | null {
  if (!tree || tree.nodes.length === 0) return null
  const words = tree.nodes.map(wordOf)
  const waiting = words.filter(word => word === "waiting").length
  // The projection's total says who needs the user, never more than what waits.
  const needsYou = Math.min(tree.totals.needs_user, waiting)
  const parts: readonly (readonly [number, string])[] = [
    [needsYou, "needs you"],
    [words.filter(word => word === "running").length, "running"],
    [waiting - needsYou, "waiting"],
    [words.filter(word => word === "done").length, "done"],
  ]
  const text = parts.filter(([n]) => n > 0)
    .map(([n, word], index) => index === 0 ? `${n} ${n === 1 ? "agent" : "agents"} ${word === "needs you" && n > 1 ? "need you" : word}` : `${n} ${word}`)
    .join(" · ")
  return { text, needsYou: needsYou > 0 }
}
