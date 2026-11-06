// MP-08 / MP-09 / MP-10 / MP-11 A6/A7 (TUI): keyboard-only operation and focus order of the stripe card and the view.
import assert from "node:assert/strict"
import test from "node:test"
import { createRoot } from "solid-js"
import type { AgentTree, AgentTreeNode, AgentTreeState } from "@chariox/kernel-client/kernel-types"
import { createCliSubAgentsComposition } from "./cli-sub-agents-composition.js"

const node = (id: string, parent: string, state: AgentTreeState, n: number): AgentTreeNode => ({
  agent_id: id, parent_agent_id: parent, kernel_id: null, alias: id, provider: "p", model: null, depth: 1, state, outcome: null, deadline_ms: null, created_at_ms: n, needs_user: false,
})
const tree: AgentTree = { root: "room", rev: 1, totals: { live: 3, running: 3, needs_user: 0 }, nodes: [node("a1", "room", "running", 1), node("a2", "a1", "running", 2), node("a3", "room", "running", 3)] }
const key = (name: string, extra: { ctrl?: boolean } = {}) => { const event = { name, ...extra, prevented: false, preventDefault() { this.prevented = true } }; return event }

test("AO-UI-01/02/03 F7 opens the card, arrows select, Enter opens the view on that panel, Ctrl+Down moves the focus, Esc closes", async () => {
  await createRoot(async dispose => {
    const handlers: Array<(event: { event: string }) => void> = []
    const client = { send: async () => ({ AgentTree: { tree } }), onKernelEvent: (handler: (event: { event: string }) => void) => { handlers.push(handler); return () => {} } }
    const session = { id: "s1", agents: [{ id: "room" }] }
    const sub = createCliSubAgentsComposition({ client: client as never, session: () => session as never, attached: () => true, focusedAgentId: () => "room", width: () => 100 })
    let draws = 0
    sub.bindRender(() => { draws += 1 })
    await sub.controller.refresh("room")
    assert.equal(sub.stripeFor("room")!.text, "3 agents running")
    assert.equal(sub.handleKey(key("x")), false, "closed: ordinary keys are the prompt's")
    assert.equal(sub.handleKey(key("f7")), true); assert.equal(sub.cardOpen(), true)
    assert.equal(sub.handleKey(key("x")), true, "the open card is modal: typing does not reach the prompt")
    assert.equal(sub.handleKey(key("c", { ctrl: true })), false, "Ctrl+C still stops or exits")
    sub.handleKey(key("down")); sub.handleKey(key("down"))
    assert.deepEqual(sub.cardRows().map(row => row.focused), [false, false, true])
    sub.handleKey(key("return"))
    assert.deepEqual([sub.cardOpen(), sub.controller.viewOpen, sub.controller.focusedPanelId], [false, true, "a3"])
    assert.equal(sub.handleKey(key("up", { ctrl: true })), true); assert.equal(sub.controller.focusedPanelId, "a2")
    assert.equal(sub.handleKey(key("up", { ctrl: true })), true); assert.equal(sub.controller.focusedPanelId, "a1")
    assert.equal(sub.handleKey(key("up")), false, "plain Up stays the prompt's history")
    sub.handleKey(key("f7")); assert.equal(sub.cardOpen(), true)
    sub.handleKey(key("f7")); assert.equal(sub.cardOpen(), false, "F7 again closes the card")
    sub.handleKey(key("f7")); sub.handleKey(key("escape")); assert.equal(sub.cardOpen(), false)
    assert.ok(draws > 0, "every state change redraws the stack and the card")
    dispose()
  })
})

test("AO-UI-01 nothing is handled while detached or with no sub-agents", () => {
  createRoot(dispose => {
    const client = { send: async () => ({}), onKernelEvent: () => () => {} }
    const sub = createCliSubAgentsComposition({ client: client as never, session: () => ({ id: "s1", agents: [] }) as never, attached: () => false, focusedAgentId: () => null, width: () => 80 })
    assert.equal(sub.handleKey(key("f7")), false)
    assert.equal(sub.stripeFor("room"), null)
    dispose()
  })
})
