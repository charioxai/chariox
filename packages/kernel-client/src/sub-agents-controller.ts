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
  /** The newest revision read per root, kept after its tree empties so a late older read cannot bring ended agents back. */
  private revs = new Map<string, number>()
  private readonly collapsed = new Set<string>()
  private root: string | null = null
  /** The focused agent the card and view were asked to show; kept until its tree arrives or the tree it had goes. */
  private requested: string | null = null
  private card = false
  private cardIndex = 0
  private view = false
  private focused: string | null = null
  private generation = 0
  /** Bumped for a root when it leaves the room: a read of it that was already on its way is dropped. */
  private epochs = new Map<string, number>()
  private reading = new Map<string, number>()

  constructor(private readonly deps: SubAgentsDeps) {}

  /** The room agent whose sub-agents the card and view show: the focused one if it has any, else the first that has. */
  select(focusedAgentId: string | null): void {
    this.requested = focusedAgentId
    this.reconcile(!focusedAgentId || !this.trees.has(focusedAgentId))
  }

  /** Reads the tree of one agent again; a failed read keeps the last state. */
  async refresh(agentId: string): Promise<void> {
    const session = this.deps.sessionId()
    if (!session) return
    const generation = this.generation
    const epoch = this.epochs.get(agentId) ?? 0
    this.reading.set(agentId, (this.reading.get(agentId) ?? 0) + 1)
    let response: AgentTreeResponse | undefined
    try { response = await this.deps.send<AgentTreeResponse>(listDescendantsRequest(session, agentId)) } catch { return } finally {
      const left = (this.reading.get(agentId) ?? 1) - 1
      if (left > 0) this.reading.set(agentId, left); else this.reading.delete(agentId)
    }
    if (generation !== this.generation || epoch !== (this.epochs.get(agentId) ?? 0) || !response?.AgentTree?.tree) return
    this.apply(response.AgentTree.tree)
  }

  /** The room agents there are now. A root that left them (deleted, or placed under another agent) loses its tree and its
   * revision, and a read of it still on its way is dropped; a later agent of the same id starts fresh. */
  retainRoots(rootIds: readonly string[]): void {
    const keep = new Set(rootIds)
    const gone = [...new Set([...this.trees.keys(), ...this.revs.keys(), ...this.reading.keys()])].filter(root => !keep.has(root))
    if (gone.length === 0) return
    for (const root of gone) {
      this.epochs.set(root, (this.epochs.get(root) ?? 0) + 1)
      this.trees.delete(root); this.revs.delete(root)
      if (this.requested === root) this.requested = null
      if (this.root === root) { this.card = false; this.view = false }
    }
    this.reconcile()
  }

  apply(tree: AgentTree): void {
    const known = this.trees.get(tree.root)
    const newest = this.revs.get(tree.root)
    if (newest !== undefined && tree.rev <= newest) return
    this.revs.set(tree.root, tree.rev)
    if (tree.nodes.length === 0) {
      this.trees.delete(tree.root)
      if (known && this.requested === tree.root) this.requested = null
    } else this.trees.set(tree.root, tree)
    this.reconcile()
  }

  /** The requested agent once it has a tree; until then the first that has one when the request just changed, else the
   * agent already shown. */
  private reconcile(requestChanged = false): void {
    const next = this.requested && this.trees.has(this.requested) ? this.requested
      : !requestChanged && this.root && this.trees.has(this.root) ? this.root
      : [...this.trees.keys()][0] ?? null
    if (next !== this.root) { this.root = next; this.cardIndex = 0; this.focused = null }
    this.settle()
  }

  /** A new room: nothing of the old one stays. */
  reset(): void {
    this.generation += 1
    this.trees = new Map(); this.revs = new Map(); this.epochs = new Map(); this.collapsed.clear(); this.root = null; this.requested = null; this.card = false; this.cardIndex = 0; this.view = false; this.focused = null
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

  /** The name a sub-agent goes by in the room agent's trees; null while the trees do not list it. */
  alias(agentId: string): string | null {
    for (const tree of this.trees.values()) {
      const node = tree.nodes.find(candidate => candidate.agent_id === agentId)
      if (node) return node.alias
    }
    return null
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
