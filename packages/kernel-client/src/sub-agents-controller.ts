// The room agents' sub-agent trees and what the user has open on them: the stripe, the list card and the
// Sub-agents view. Status only: nothing here reads or answers a decision.
import type { AgentTree, AgentTreeResponse } from "./kernel-types-runtime.js"
import { listDescendantsRequest } from "./ipc-agent-requests.js"
import { subAgentPanels, subAgentRows, subAgentStripe, visibleSubAgentRows, type SubAgentPanel, type SubAgentRow, type SubAgentStripe } from "./sub-agent-list.js"

export interface SubAgentsDeps {
  send<T>(request: unknown): Promise<T>
  /** Called after every state change that a screen shows. */
  changed(): void
  sessionId(): string | null
}

export class SubAgentsController {
  private trees = new Map<string, AgentTree>()
  private readonly collapsed = new Set<string>()
  private root: string | null = null
  private card = false
  private cardIndex = 0
  private view = false
  private focused: string | null = null
  private generation = 0

  constructor(private readonly deps: SubAgentsDeps) {}

  /** The room agent whose sub-agents the card and view show: the focused one if it has any, else the first that has. */
  select(focusedAgentId: string | null): void {
    const next = focusedAgentId && this.trees.has(focusedAgentId) ? focusedAgentId : [...this.trees.keys()][0] ?? null
    if (next !== this.root) { this.root = next; this.cardIndex = 0; this.focused = null; this.settle() }
  }

  /** Reads the tree of one agent again; a failed read keeps the last state. */
  async refresh(agentId: string): Promise<void> {
    const session = this.deps.sessionId()
    if (!session) return
    const generation = this.generation
    let response: AgentTreeResponse | undefined
    try { response = await this.deps.send<AgentTreeResponse>(listDescendantsRequest(session, agentId)) } catch { return }
    if (generation !== this.generation || !response?.AgentTree?.tree) return
    this.apply(response.AgentTree.tree)
  }

  apply(tree: AgentTree): void {
    const known = this.trees.get(tree.root)
    if (known && tree.rev < known.rev) return
    if (tree.nodes.length === 0) this.trees.delete(tree.root); else this.trees.set(tree.root, tree)
    if (this.root === null || this.root === tree.root) this.root = this.trees.has(tree.root) ? tree.root : [...this.trees.keys()][0] ?? null
    this.settle()
  }

  /** A new room: nothing of the old one stays. */
  reset(): void {
    this.generation += 1
    this.trees = new Map(); this.collapsed.clear(); this.root = null; this.card = false; this.cardIndex = 0; this.view = false; this.focused = null
    this.deps.changed()
  }

  private current(): AgentTree | null { return this.root ? this.trees.get(this.root) ?? null : null }

  /** The view exists only while a sub-agent is live; the card only while the list has rows. When the last one ends,
   * the view closes like any room view and the room goes back to its agents. */
  private settle(): void {
    const tree = this.current()
    if (!tree || tree.nodes.length === 0) this.card = false
    if (!tree || tree.totals.live <= 0) { this.view = false; this.focused = null }
    else if (this.view && !this.panels().some(panel => panel.id === this.focused)) this.focused = this.panels()[0]?.id ?? null
    this.cardIndex = Math.max(0, Math.min(this.cardIndex, this.lines().length - 1))
    this.deps.changed()
  }

  /** The sub-agents stripe of one agent: null for every agent but the room agent that has sub-agents. */
  stripe(agentId: string | null | undefined): SubAgentStripe | null {
    return agentId ? subAgentStripe(this.trees.get(agentId) ?? null) : null
  }

  lines(): SubAgentRow[] { const tree = this.current(); return tree ? visibleSubAgentRows(subAgentRows(tree), this.collapsed) : [] }
  panels(): SubAgentPanel[] { const tree = this.current(); return tree ? subAgentPanels(tree) : [] }
  get cardOpen(): boolean { return this.card }
  get selectedIndex(): number { return this.cardIndex }
  get viewOpen(): boolean { return this.view }
  get focusedPanelId(): string | null { return this.focused }
  get available(): boolean { return this.current() !== null }
  isCollapsed(id: string): boolean { return this.collapsed.has(id) }

  openCard(): void {
    if (!this.current()) return
    this.card = true; this.cardIndex = Math.min(this.cardIndex, this.lines().length - 1); this.deps.changed()
  }
  closeCard(): void { if (this.card) { this.card = false; this.deps.changed() } }

  moveCard(by: number): void {
    const count = this.lines().length
    if (count === 0) return
    this.cardIndex = (this.cardIndex + by + count) % count
    this.deps.changed()
  }

  /** Right expands the selected row, Left collapses it. */
  expandSelected(expanded: boolean): void {
    const row = this.lines()[this.cardIndex]
    if (!row || row.children.length === 0) return
    if (expanded) this.collapsed.delete(row.id); else this.collapsed.add(row.id)
    this.deps.changed()
  }

  /** A row of the card: opens the view if it is closed and focuses that panel; if open, moves the focus.
   * With no live sub-agent there is no view: the card only closes. */
  chooseRow(id: string | undefined = this.lines()[this.cardIndex]?.id): void {
    this.card = false
    const tree = this.current()
    if (id && tree && tree.totals.live > 0 && this.panels().some(panel => panel.id === id)) { this.view = true; this.focused = id }
    this.deps.changed()
  }

  /** Moves the focus between the panels, in card order; it stops at the ends. */
  moveFocus(by: number): void {
    const panels = this.panels()
    if (!this.view || panels.length === 0) return
    const at = panels.findIndex(panel => panel.id === this.focused)
    this.focused = panels[Math.max(0, Math.min(panels.length - 1, (at < 0 ? 0 : at) + by))]!.id
    this.deps.changed()
  }
}
