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

test("AO-UI-01 a click on a stripe selects that agent's sub-agents and opens the card on them", async () => {
  await createRoot(async dispose => {
    const other: AgentTree = { ...tree, root: "other", nodes: [node("b1", "other", "running", 1)], totals: { live: 1, running: 1, needs_user: 0 } }
    const client = { send: async (request: { ListDescendants: { agent_id: string } }) => ({ AgentTree: { tree: request.ListDescendants.agent_id === "other" ? other : tree } }), onKernelEvent: () => () => {} }
    const session = { id: "s1", agents: [{ id: "room" }, { id: "other" }] }
    const sub = createCliSubAgentsComposition({ client: client as never, session: () => session as never, attached: () => true, focusedAgentId: () => "room", width: () => 100 })
    await sub.controller.refresh("room"); await sub.controller.refresh("other")
    sub.stripeFor("other")!.open()
    assert.equal(sub.cardOpen(), true)
    assert.deepEqual(sub.controller.lines().map(row => row.id), ["b1"])
    dispose()
  })
})

test("A3/A4 a decision a sub-agent asks in the room agent's panel is labelled with the sub-agent's name", async () => {
  await createRoot(async dispose => {
    const client = { send: async () => ({ AgentTree: { tree } }), onKernelEvent: () => () => {} }
    const sub = createCliSubAgentsComposition({ client: client as never, session: () => ({ id: "s1", agents: [{ id: "room" }] }) as never, attached: () => true, focusedAgentId: () => "room", width: () => 100 })
    await sub.controller.refresh("room")
    const card = (agent_id: string, shown_in_agent_id: string | null) => ({ id: "card", agent_id, shown_in_agent_id }) as never
    assert.equal(sub.askingAgentLabel(card("a2", "room")), "a2")
    assert.equal(sub.askingAgentLabel(card("unlisted", "room")), "sub-agent", "a child the tree does not list yet is still not the room agent")
    assert.equal(sub.askingAgentLabel(card("room", null)), null, "the room agent's own decision names nobody")
    assert.equal(sub.askingAgentLabel(card("room", "room")), null)
    dispose()
  })
})

// MP-08 / MP-09 / MP-10 / MP-11 subview: a root that leaves the room's agents (deleted, or placed under another agent)
// takes its cached tree, its open view and its stripe with it, whatever the room id.
const flush = () => new Promise(resolve => setTimeout(resolve, 0))
const rooms = () => {
  const handlers: Array<(event: { event: string }) => void> = []
  const answers: Record<string, () => Promise<AgentTree>> = {}
  const client = {
    send: async (request: { ListDescendants: { agent_id: string } }) => ({ AgentTree: { tree: await answers[request.ListDescendants.agent_id]!() } }),
    onKernelEvent: (handler: (event: { event: string }) => void) => { handlers.push(handler); return () => {} },
  }
  const session = { id: "s1", agents: [{ id: "a" }, { id: "b" }] as Array<{ id: string }> }
  const sub = createCliSubAgentsComposition({ client: client as never, session: () => session as never, attached: () => true, focusedAgentId: () => "a", width: () => 100 })
  const roomChanged = async () => { handlers.forEach(handler => handler({ event: "session_snapshot" })); await flush() }
  const tree = (root: string, rev: number, nodes: AgentTreeNode[]): AgentTree => ({ root, rev, nodes, totals: { live: nodes.length, running: nodes.length, needs_user: 0 } })
  return { sub, session, answers, roomChanged, tree }
}

test("AO-UI-02 deleting a root drops its cached tree, its stripe and its open view", async () => {
  await createRoot(async dispose => {
    const { sub, session, answers, roomChanged, tree } = rooms()
    answers.a = async () => tree("a", 1, [node("a1", "a", "running", 1)])
    answers.b = async () => tree("b", 1, [])
    await sub.controller.refresh("a"); await sub.controller.refresh("b")
    sub.stripeFor("a")!.open(); sub.controller.chooseRow("a1")
    assert.equal(sub.viewVisible(), true)
    session.agents = [{ id: "b" }]
    await roomChanged()
    assert.deepEqual([sub.stripeFor("a"), sub.viewVisible(), sub.cardOpen(), sub.controller.available], [null, false, false, false])
    dispose()
  })
})

test("AO-UI-02 a root placed under another agent leaves the room's trees and the other root's tree shows it", async () => {
  await createRoot(async dispose => {
    const { sub, session, answers, roomChanged, tree } = rooms()
    answers.a = async () => tree("a", 1, [node("a1", "a", "running", 1)])
    answers.b = async () => tree("b", 1, [])
    await sub.controller.refresh("a"); await sub.controller.refresh("b")
    session.agents = [{ id: "b" }]
    answers.b = async () => tree("b", 2, [node("a", "b", "running", 1), node("a1", "a", "running", 2)])
    await roomChanged()
    assert.equal(sub.stripeFor("a"), null, "a is no room agent any more")
    assert.deepEqual(sub.stripeFor("b")!.text, "2 agents running")
    assert.deepEqual(sub.controller.lines().map(row => row.id), ["a", "a1"])
    dispose()
  })
})

test("AO-UI-02 a read of a deleted root that is still on its way is dropped", async () => {
  await createRoot(async dispose => {
    const { sub, session, answers, roomChanged, tree } = rooms()
    let answer: (value: AgentTree) => void = () => {}
    answers.a = () => new Promise<AgentTree>(resolve => { answer = resolve })
    answers.b = async () => tree("b", 1, [])
    const late = sub.controller.refresh("a")
    session.agents = [{ id: "b" }]
    await roomChanged()
    answer(tree("a", 1, [node("a1", "a", "running", 1)]))
    await late
    assert.equal(sub.stripeFor("a"), null)
    dispose()
  })
})

test("AO-UI-02 a root deleted and created again under the same id reads fresh, whatever its old revision was", async () => {
  await createRoot(async dispose => {
    const { sub, session, answers, roomChanged, tree } = rooms()
    answers.a = async () => tree("a", 9, [node("a1", "a", "running", 1)])
    answers.b = async () => tree("b", 1, [])
    await sub.controller.refresh("a")
    session.agents = [{ id: "b" }]
    await roomChanged()
    session.agents = [{ id: "a" }, { id: "b" }]
    answers.a = async () => tree("a", 1, [node("a2", "a", "running", 1)])
    await roomChanged()
    assert.deepEqual(sub.controller.lines().map(row => row.id), ["a2"])
    dispose()
  })
})

test("ordinary Room events do not reread trees; tree changes and new roots do", async () => {
  await createRoot(async dispose => {
    const handlers: Array<(event: Record<string, unknown>) => void> = []
    let reads = 0
    const session = { id: "s1", agents: [{ id: "room" }] }
    const client = {
      send: async (request: { ListDescendants: { agent_id: string } }) => { reads++; return { AgentTree: { tree: { ...tree, root: request.ListDescendants.agent_id, rev: reads } } } },
      onKernelEvent: (handler: (event: Record<string, unknown>) => void) => { handlers.push(handler); return () => {} },
    }
    const sub = createCliSubAgentsComposition({ client: client as never, session: () => session as never, attached: () => true, focusedAgentId: () => "room", width: () => 100 })
    await sub.controller.refresh("room"); await flush()
    const before = reads
    const emit = async (event: Record<string, unknown>) => { handlers.forEach(handler => handler(event)); await flush() }
    for (const event of ["session_snapshot", "agent_activity_changed", "provider_run_changed", "runtime_interactions_changed"]) await emit({ event, session_id: "s1" })
    assert.equal(reads, before, "unchanged membership and activity leave the trees alone")
    await emit({ event: "agent_tree_changed", session_id: "other", root: "room", rev: 10 })
    assert.equal(reads, before)
    await emit({ event: "agent_tree_changed", session_id: "s1", root: "room", rev: 10 })
    assert.equal(reads, before + 1)
    session.agents.push({ id: "new-root" })
    await emit({ event: "session_snapshot", session_id: "s1" })
    assert.equal(reads, before + 3, "changed membership may move children between roots")
    dispose()
  })
})
