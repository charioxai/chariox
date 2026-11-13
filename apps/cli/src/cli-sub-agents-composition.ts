// MP-08 / MP-09 / MP-10 / MP-11 A6/A7: wires the sub-agents stripe, list card and view to the session. The kernel's
// tree revision is the source for reads; Room snapshots only reconcile membership. This file also routes the keys.
import { createEffect, createSignal, onCleanup, untrack } from "solid-js"
import { SubAgentsController } from "@chariox/kernel-client/sub-agents-controller"
import { dialogTextWidth } from "./cli-dialog-panel-size.js"
import type { LocalIpcClient } from "./ipc.js"
import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import { subAgentsCardRows } from "./sub-agents-card-view.js"

type Key = { name: string; ctrl?: boolean; meta?: boolean; shift?: boolean; eventType?: string; preventDefault?(): void; stopPropagation?(): void }

export function createCliSubAgentsComposition(deps: {
  client: LocalIpcClient
  session(): RuntimeSession
  attached(): boolean
  focusedAgentId(): string | null
  width(): number
}) {
  const [revision, setRevision] = createSignal(0)
  let draw: () => void = () => {}
  const controller = new SubAgentsController({
    send: request => deps.client.send(request),
    sessionId: () => (deps.attached() ? deps.session().id : null),
    changed: () => { setRevision(value => value + 1); draw() },
  })

  let roots = new Set<string>()
  const reconcileRoots = async () => {
    const ids = deps.attached() ? (deps.session().agents ?? []).map((agent: { id: string }) => agent.id) : []
    controller.retainRoots(ids)
    const changed = ids.length !== roots.size || ids.some(id => !roots.has(id))
    roots = new Set(ids)
    if (changed) await Promise.all(ids.map(id => controller.refresh(id)))
  }

  let attachedTo: string | null = null
  createEffect(() => {
    const id = deps.attached() ? deps.session().id : null
    untrack(() => {
      if (id === attachedTo) return
      attachedTo = id
      roots.clear()
      controller.reset()
      if (id) void reconcileRoots()
    })
  })
  createEffect(() => { const focused = deps.focusedAgentId(); untrack(() => controller.select(focused)) })
  onCleanup(deps.client.onKernelEvent(event => {
    if (!deps.attached()) return
    if (event.event === "session_snapshot") { void reconcileRoots(); return }
    if (event.event === "agent_tree_changed" && event.session_id === deps.session().id && roots.has(event.root)) void controller.refresh(event.root)
  }))

  const cardOpen = () => { revision(); return controller.cardOpen }
  const viewVisible = () => { revision(); return deps.attached() && controller.viewOpen }
  const consume = (event: Key) => { event.preventDefault?.(); event.stopPropagation?.(); return true }

  return {
    controller, revision, viewVisible, cardOpen,
    /** Re-draws the screens that show the controller's state (the stack and the card), set once they exist. */
    bindRender(render: () => void) { draw = render },
    /** Names the sub-agent that asks a decision shown in its room agent's panel; null when the shown agent asks itself. */
    askingAgentLabel(interaction: RuntimeInteraction): string | null {
      const asking = interaction.agent_id
      if (!asking || !interaction.shown_in_agent_id || interaction.shown_in_agent_id === asking) return null
      revision()
      return controller.alias(asking) ?? "sub-agent"
    },
    stripeFor(agentId: string | null | undefined) {
      const stripe = deps.attached() ? controller.stripe(agentId) : null
      return stripe && { ...stripe, open: () => { controller.select(agentId ?? null); controller.openCard() } }
    },
    cardRows: () => subAgentsCardRows(controller.lines(), controller.selectedIndex, new Set(controller.lines().filter(row => controller.isCollapsed(row.id)).map(row => row.id)), dialogTextWidth(deps.width())),
    closeCard: () => controller.closeCard(),
    /** F7 opens the card from anywhere; with the card open the list is modal; Ctrl+Up/Down move the focus of an open view. */
    handleKey(event: Key): boolean {
      if (event.eventType === "release" || !deps.attached()) return false
      if (controller.cardOpen) {
        if (event.name === "f7" && !event.ctrl) { controller.closeCard(); return consume(event) }
        if (event.ctrl || event.meta) return false
        switch (event.name) {
          case "up": controller.moveCard(-1); break
          case "down": controller.moveCard(1); break
          case "left": controller.expandSelected(false); break
          case "right": controller.expandSelected(true); break
          case "return": case "enter": controller.chooseRow(); break
          case "escape": controller.closeCard(); break
        }
        return consume(event)
      }
      if (event.name === "f7" && !event.ctrl && controller.available) { controller.openCard(); return consume(event) }
      if (controller.viewOpen && (event.ctrl || event.meta) && (event.name === "up" || event.name === "down")) {
        controller.moveFocus(event.name === "up" ? -1 : 1)
        return consume(event)
      }
      return false
    },
  }
}
