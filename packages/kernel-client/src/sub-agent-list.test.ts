// MP-08 / MP-09 / MP-10 / MP-11 A6/A7 (TUI): what the sub-agents stripe, list card and sub-view show.
import assert from "node:assert/strict"
import test from "node:test"
import type { AgentTree, AgentTreeNode, AgentTreeState } from "./kernel-types-runtime.js"
import { subAgentPanels, subAgentRows, subAgentStripe, visibleSubAgentRows } from "./sub-agent-list.js"

const node = (id: string, parent: string | null, state: AgentTreeState, extra: Partial<AgentTreeNode> = {}): AgentTreeNode => ({
  agent_id: id, parent_agent_id: parent ?? "room-agent", kernel_id: null, alias: id, provider: "p", model: null,
  depth: 1, state, outcome: null, deadline_ms: null, created_at_ms: Number(id.replace(/\D/g, "")) || 0, needs_user: false, ...extra,
})
const tree = (nodes: AgentTreeNode[]): AgentTree => ({
  root: "room-agent", rev: 1, nodes,
  totals: { live: nodes.filter(n => ["starting", "running", "waiting", "unreachable"].includes(n.state)).length, running: nodes.filter(n => n.state === "running").length, needs_user: nodes.filter(n => n.needs_user === true).length },
})

test("AO-UI-01 no sub-agents, no stripe", () => {
  assert.equal(subAgentStripe(null), null)
  assert.equal(subAgentStripe(tree([])), null)
})

test("AO-UI-01 one agent running reads singular, six over three depths read plain counts", () => {
  assert.equal(subAgentStripe(tree([node("a1", null, "running")]))!.text, "1 agent running")
  const six = tree([
    node("a1", null, "running"), node("a2", null, "running"), node("a3", "a1", "running", { depth: 2 }),
    node("a4", "a1", "running", { depth: 2 }), node("a5", "a3", "running", { depth: 3 }), node("a6", "a2", "starting", { depth: 2 }),
  ])
  assert.equal(subAgentStripe(six)!.text, "6 agents running")
  assert.equal(subAgentStripe(six)!.needsYou, false)
})

test("AO-UI-01 needs you leads, then running, waiting and done; plural of need", () => {
  const mixed = tree([
    node("a1", null, "running"), node("a2", null, "running"), node("a3", null, "waiting", { needs_user: true }),
    node("a4", null, "waiting"), node("a5", null, "ended"), node("a6", null, "idle"),
  ])
  assert.deepEqual(subAgentStripe(mixed), { text: "1 agent needs you · 2 running · 1 waiting · 2 done", needsYou: true })
  const two = tree([node("a1", null, "waiting", { needs_user: true }), node("a2", null, "waiting", { needs_user: true })])
  assert.equal(subAgentStripe(two)!.text, "2 agents need you")
  assert.equal(subAgentStripe(tree([node("a1", null, "ended"), node("a2", null, "error"), node("a3", null, "idle")]))!.text, "3 agents done")
})

test("AO-UI-01 needs you comes from the node flag only: an ancestor-held wait reads waiting", () => {
  const t = tree([node("a1", null, "waiting", { needs_user: true }), node("a2", "a1", "waiting", { depth: 2 }), node("a3", null, "waiting")])
  assert.deepEqual(subAgentRows(t).flatMap(function walk(row): string[] { return [`${row.id}:${row.word}`, ...row.children.flatMap(walk)] }), ["a1:needs you", "a2:waiting", "a3:waiting"])
  assert.equal(subAgentStripe(t)!.text, "1 agent needs you · 2 waiting")
})

test("AO-UI-02 rows nest under their parent at every depth, in creation order, finished dimmed", () => {
  const t = tree([
    node("a3", "a1", "running", { depth: 2, created_at_ms: 30 }), node("a1", null, "running", { created_at_ms: 10 }),
    node("a2", null, "ended", { created_at_ms: 20 }), node("a4", "a3", "idle", { depth: 3, created_at_ms: 40 }),
  ])
  const rows = subAgentRows(t)
  assert.deepEqual(rows.map(r => r.id), ["a1", "a2"])
  assert.deepEqual(rows[0]!.children.map(r => r.id), ["a3"])
  assert.deepEqual(rows[0]!.children[0]!.children.map(r => [r.id, r.word, r.dimmed]), [["a4", "done", true]])
  assert.deepEqual(rows.map(r => [r.word, r.dimmed]), [["running", false], ["done", true]])
  const flat = visibleSubAgentRows(rows, new Set())
  assert.deepEqual(flat.map(r => [r.id, r.depth]), [["a1", 0], ["a3", 1], ["a4", 2], ["a2", 0]])
  assert.deepEqual(visibleSubAgentRows(rows, new Set(["a1"])).map(r => r.id), ["a1", "a2"], "a collapsed row hides its descendants")
  // the same projection in another arrival order lists the same order
  assert.deepEqual(subAgentRows({ ...t, nodes: [...t.nodes].reverse() }).map(r => r.id), ["a1", "a2"])
})

test("AO-UI-02 a node whose parent is unknown is listed on top, and a loop of parents still lists every agent", () => {
  const orphan = tree([node("a1", "gone", "running")])
  assert.deepEqual(subAgentRows(orphan).map(r => r.id), ["a1"])
  const loop = tree([node("a1", "a2", "running"), node("a2", "a1", "running")])
  assert.deepEqual(visibleSubAgentRows(subAgentRows(loop), new Set()).map(r => r.id).sort(), ["a1", "a2"])
})

test("AO-UI-03 one panel per sub-agent in card order, status only, a held decision reads waiting", () => {
  const t = tree([
    node("a1", null, "running", { created_at_ms: 1 }), node("a2", "a1", "waiting", { depth: 2, needs_user: true, created_at_ms: 2 }),
    node("a3", null, "ended", { created_at_ms: 3 }),
  ])
  assert.deepEqual(subAgentPanels(t), [
    { id: "a1", name: "a1", word: "running", dimmed: false, depth: 0 },
    { id: "a2", name: "a2", word: "waiting", dimmed: false, depth: 1 },
    { id: "a3", name: "a3", word: "done", dimmed: true, depth: 0 },
  ])
})

test("a node without an alias is named by its id", () => {
  assert.equal(subAgentRows(tree([node("a1", null, "running", { alias: null })]))[0]!.name, "a1")
})
