// MP-08 / MP-09 / MP-10 / MP-11 A6/A7 (TUI): the stripe, list card and Sub-agents view lifecycle.
import assert from "node:assert/strict"
import test from "node:test"
import { readFileSync } from "node:fs"
import type { AgentTree, AgentTreeNode, AgentTreeState } from "./kernel-types-runtime.js"
import { SubAgentsController } from "./sub-agents-controller.js"

const node = (id: string, parent: string, state: AgentTreeState, n: number, extra: Partial<AgentTreeNode> = {}): AgentTreeNode => ({
  agent_id: id, parent_agent_id: parent, kernel_id: null, alias: id, provider: "p", model: null, depth: 1, state, outcome: null,
  deadline_ms: null, created_at_ms: n, needs_user: false, ...extra,
})
const tree = (rev: number, nodes: AgentTreeNode[], root = "room"): AgentTree => ({
  root, rev, nodes, totals: { live: nodes.filter(x => ["starting", "running", "waiting", "unreachable"].includes(x.state)).length, running: 0, needs_user: nodes.filter(x => x.needs_user).length },
})
const make = (answers: Record<string, AgentTree> = {}) => {
  const requests: unknown[] = []
  let changes = 0
  const controller = new SubAgentsController({
    send: async <T>(request: unknown) => { requests.push(request); const root = (request as { ListDescendants: { agent_id: string } }).ListDescendants.agent_id; return { AgentTree: { tree: answers[root]! } } as T },
    changed: () => { changes += 1 }, sessionId: () => "s1",
  })
  return { controller, requests, changes: () => changes, answers }
}
const three = (rev = 1) => tree(rev, [node("a1", "room", "running", 1), node("a2", "a1", "running", 2), node("a3", "room", "waiting", 3, { needs_user: true })])

test("AO-UI-01 the stripe belongs to the room agent that has sub-agents, nothing else", async () => {
  const { controller, requests } = make({ room: three() })
  assert.equal(controller.stripe("room"), null)
  await controller.refresh("room")
  assert.deepEqual(requests, [{ ListDescendants: { session_id: "s1", agent_id: "room" } }])
  assert.equal(controller.stripe("room")!.text, "1 agent needs you · 2 running")
  assert.equal(controller.stripe("another"), null)
  assert.equal(controller.stripe(null), null)
})

test("AO-UI-02 the card lists the hierarchy; Right/Left expand and collapse; the selection wraps in order", async () => {
  const { controller } = make({ room: three() })
  await controller.refresh("room")
  controller.openCard()
  assert.equal(controller.cardOpen, true)
  assert.deepEqual(controller.lines().map(r => [r.id, r.depth]), [["a1", 0], ["a2", 1], ["a3", 0]])
  controller.expandSelected(false)
  assert.deepEqual(controller.lines().map(r => r.id), ["a1", "a3"])
  controller.expandSelected(true)
  controller.moveCard(-1)
  assert.equal(controller.lines()[controller.selectedIndex]!.id, "a3", "up from the first row wraps to the last")
  controller.moveCard(1)
  assert.equal(controller.lines()[controller.selectedIndex]!.id, "a1")
})

test("AO-UI-03 a row opens the view if it is closed and focuses its panel; with the view open it moves the focus", async () => {
  const { controller } = make({ room: three() })
  await controller.refresh("room")
  assert.equal(controller.viewOpen, false, "arrival does not open the view")
  controller.openCard(); controller.moveCard(1)
  controller.chooseRow()
  assert.deepEqual([controller.cardOpen, controller.viewOpen, controller.focusedPanelId], [false, true, "a2"])
  controller.openCard()
  controller.chooseRow("a3")
  assert.deepEqual([controller.viewOpen, controller.focusedPanelId], [true, "a3"])
  controller.moveFocus(-1); assert.equal(controller.focusedPanelId, "a2")
  controller.moveFocus(-5); assert.equal(controller.focusedPanelId, "a1", "the focus stops at the first panel")
  controller.moveFocus(9); assert.equal(controller.focusedPanelId, "a3", "and at the last")
})

test("AO-UI-03 a node finishing while its panel is focused keeps the panel and the focus", async () => {
  const { controller } = make({ room: three(1) })
  await controller.refresh("room"); controller.chooseRow("a2")
  controller.apply(tree(2, [node("a1", "room", "running", 1), node("a2", "a1", "ended", 2), node("a3", "room", "waiting", 3)]))
  assert.deepEqual([controller.viewOpen, controller.focusedPanelId], [true, "a2"])
  assert.deepEqual(controller.panels().map(p => [p.id, p.word, p.dimmed]), [["a1", "running", false], ["a2", "done", true], ["a3", "waiting", false]])
})

test("AO-UI-03 the last live sub-agent ending closes the view; the finished rows stay on the card", async () => {
  const { controller } = make({ room: three(1) })
  await controller.refresh("room"); controller.chooseRow("a1")
  controller.apply(tree(2, [node("a1", "room", "ended", 1), node("a2", "a1", "ended", 2), node("a3", "room", "idle", 3)]))
  assert.deepEqual([controller.viewOpen, controller.focusedPanelId], [false, null])
  assert.equal(controller.stripe("room")!.text, "3 agents done")
  controller.openCard(); controller.chooseRow("a1")
  assert.deepEqual([controller.cardOpen, controller.viewOpen], [false, false], "with none live a row only closes the card")
})

// The kernel's own answer once every child finished (the Rust projection test asserts this file): the totals are the
// kernel's, not recomputed here, so the two sides cannot drift apart.
const finishedByKernel = (rev: number): AgentTree => ({ ...JSON.parse(readFileSync(new URL("../../../fixtures/agent-tree-finished.json", import.meta.url), "utf8")) as AgentTree, rev })

test("AO-UI-03 the kernel's tree of finished children closes the view and keeps it closed", async () => {
  const { controller } = make({ room: three(1) })
  await controller.refresh("room"); controller.chooseRow("a1")
  assert.equal(controller.viewOpen, true)
  controller.apply(finishedByKernel(2))
  assert.deepEqual([controller.viewOpen, controller.focusedPanelId], [false, null], "the last child finishing closes the view")
  controller.openCard(); controller.chooseRow("done")
  assert.deepEqual([controller.cardOpen, controller.viewOpen], [false, false], "a done row does not reopen it")
})

test("AO-UI-02 the parent's task ending drops every sub-agent: stripe, card and view go", async () => {
  const { controller } = make({ room: three(1) })
  await controller.refresh("room"); controller.openCard(); controller.chooseRow("a2"); controller.openCard()
  controller.apply(tree(2, []))
  assert.deepEqual([controller.stripe("room"), controller.cardOpen, controller.viewOpen, controller.available], [null, false, false, false])
})

test("an older revision never replaces a newer one, and a failed read keeps the last state", async () => {
  const { controller, answers } = make({ room: three(5) })
  await controller.refresh("room")
  controller.apply(tree(4, []))
  assert.equal(controller.stripe("room")!.text, "1 agent needs you · 2 running")
  delete answers.room
  await controller.refresh("room")
  assert.equal(controller.stripe("room")!.text, "1 agent needs you · 2 running")
})

test("AO-UI-02 a root that leaves the room's agents loses its tree, its view and its revision", async () => {
  const { controller } = make({ room: three(7) })
  await controller.refresh("room"); controller.chooseRow("a1")
  controller.apply(tree(2, [node("b1", "other", "running", 1)], "other"))
  controller.retainRoots(["other"])
  assert.deepEqual([controller.stripe("room"), controller.viewOpen, controller.cardOpen, controller.available], [null, false, false, true], "the view of a vanished agent does not turn into another agent's")
  assert.deepEqual(controller.lines().map(row => row.id), ["b1"])
  controller.retainRoots(["room", "other"])
  controller.apply(tree(1, [node("c1", "room", "running", 1)]))
  assert.equal(controller.stripe("room")!.text, "1 agent running", "an agent that took the id again reads fresh, below the old revision")
})

test("AO-UI-02 a read of a root that left the room while it was on its way is dropped", async () => {
  let answer: (value: unknown) => void = () => {}
  const controller = new SubAgentsController({ send: <T>() => new Promise<T>(resolve => { answer = resolve as (value: unknown) => void }), changed: () => {}, sessionId: () => "s1" })
  const late = controller.refresh("room")
  controller.retainRoots([])
  answer({ AgentTree: { tree: three(1) } })
  await late
  assert.equal(controller.available, false)
})

test("a read answered after the room was left is dropped", async () => {
  const { controller } = make({ room: three() })
  const pending = controller.refresh("room")
  controller.reset()
  await pending
  assert.equal(controller.available, false)
})

test("the card and view follow the focused room agent that has sub-agents", async () => {
  const { controller } = make({ room: three(), other: tree(1, [node("b1", "other", "running", 1)], "other") })
  await controller.refresh("room"); await controller.refresh("other")
  controller.select("other"); assert.deepEqual(controller.lines().map(r => r.id), ["b1"])
  controller.select("nobody"); assert.deepEqual(controller.lines().map(r => r.id), ["a1", "a2", "a3"])
})

const delayed = () => {
  const gates = new Map<string, () => void>()
  const requests: string[] = []
  const answers: Record<string, AgentTree> = {
    a: tree(1, [node("a1", "a", "running", 1)], "a"),
    b: tree(1, [node("b1", "b", "running", 1)], "b"),
  }
  const controller = new SubAgentsController({
    send: async <T>(request: unknown) => {
      const root = (request as { ListDescendants: { agent_id: string } }).ListDescendants.agent_id
      requests.push(root)
      await new Promise<void>(resolve => gates.set(root, resolve))
      return { AgentTree: { tree: answers[root]! } } as T
    },
    changed: () => {}, sessionId: () => "s1",
  })
  return { controller, gates, requests }
}

for (const order of [["a", "b"], ["b", "a"]] as const) {
  test(`the focused root chosen before the trees arrive wins when the reads finish ${order.join(" then ")}`, async () => {
    const { controller, gates } = delayed()
    const reads = [controller.refresh("a"), controller.refresh("b")]
    controller.select("b")
    for (const id of order) { gates.get(id)!(); await Promise.resolve(); await Promise.resolve() }
    await Promise.all(reads)
    assert.deepEqual(controller.lines().map(r => r.id), ["b1"], "the children of the focused agent show")
    assert.equal(controller.stripe("b")!.text, "1 agent running")
  })
}

test("selecting a root whose tree never arrives keeps the first known tree, and the request settles when that tree arrives", async () => {
  const { controller } = make({ room: three(), late: tree(1, [node("b1", "late", "running", 1)], "late") })
  await controller.refresh("room")
  controller.select("late")
  assert.deepEqual(controller.lines().map(r => r.id), ["a1", "a2", "a3"])
  await controller.refresh("late")
  assert.deepEqual(controller.lines().map(r => r.id), ["b1"])
})

test("the request ends with the tree it asked for: the root falls back and a later arrival does not pull the view back", async () => {
  const { controller } = make({ room: three(), other: tree(1, [node("b1", "other", "running", 1)], "other") })
  await controller.refresh("room"); await controller.refresh("other")
  controller.select("other")
  controller.apply(tree(2, [], "other"))
  assert.deepEqual(controller.lines().map(r => r.id), ["a1", "a2", "a3"])
  controller.apply(tree(3, [node("b1", "other", "running", 1)], "other"))
  assert.deepEqual(controller.lines().map(r => r.id), ["a1", "a2", "a3"])
})

test("a late read older than the empty tree that closed the stripe does not bring the agents back", () => {
  const { controller } = make()
  controller.apply(three(5))
  assert.equal(controller.stripe("room")!.text.length > 0, true)
  controller.apply(tree(6, []))
  assert.ok(controller.stripe("room") === null, "the last sub-agent ended")
  // The read made before the change answers after it.
  controller.apply(three(5))
  assert.ok(controller.stripe("room") === null, "a read older than rev 6 must not show agents again")
})

test("repeated tree revisions do not redraw unchanged stripes, including empty trees", () => {
  for (const snapshot of [three(), tree(1, [])]) {
    const h = make()
    h.controller.apply(snapshot)
    const changes = h.changes()
    h.controller.apply(structuredClone(snapshot))
    assert.equal(h.changes(), changes)
    h.controller.apply({ ...snapshot, rev: 2 })
    assert.equal(h.changes(), changes + 1)
  }
})
