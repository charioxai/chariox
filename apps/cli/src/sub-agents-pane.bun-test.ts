// MP-08 / MP-09 / MP-10 / MP-11 A7 (TUI): the Sub-agents view shows one status panel per sub-agent and nothing to decide.
import assert from "node:assert/strict"
import test from "node:test"
import { createComponent, createSignal } from "solid-js"
import { testRender } from "@opentui/solid"
import type { AgentTree, AgentTreeNode, AgentTreeState } from "@chariox/kernel-client/kernel-types"
import { SubAgentsController } from "@chariox/kernel-client/sub-agents-controller"
import { SubAgentsPane } from "./sub-agents-pane.js"

const node = (id: string, parent: string, state: AgentTreeState, n: number, needs_user = false): AgentTreeNode => ({
  agent_id: id, parent_agent_id: parent, kernel_id: null, alias: id, provider: "p", model: null, depth: 1, state, outcome: null, deadline_ms: null, created_at_ms: n, needs_user,
})
const live = (nodes: AgentTreeNode[], rev = 1): AgentTree => ({ root: "room", rev, nodes, totals: { live: nodes.filter(n => ["running", "waiting"].includes(n.state)).length, running: 0, needs_user: nodes.filter(n => n.needs_user).length } })

for (const width of [32, 36]) test(`AO-UI-03 one panel per sub-agent at ${width} columns: name and one word, a held decision reads waiting`, async () => {
  const [revision, setRevision] = createSignal(0)
  const controller = new SubAgentsController({ send: async () => { throw new Error("no read") }, changed: () => setRevision(value => value + 1), sessionId: () => "s" })
  controller.apply(live([node("researcher", "room", "running", 1), node("reader", "researcher", "waiting", 2, true), node("writer", "room", "ended", 3)]))
  controller.chooseRow("reader")
  const harness = await testRender(() => createComponent(SubAgentsPane, { controller, revision, width, onFocus: id => controller.chooseRow(id) }), { width, height: 18, useThread: false })
  try {
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Sub-agents 3/)
    const rows = frame.split("\n")
    const line = (name: string) => rows.find(row => row.includes(name))!
    assert.match(line("researcher"), /researcher\s+running/); assert.match(line("reader"), /reader\s+waiting/); assert.match(line("writer"), /writer\s+done/)
    assert.ok(!/needs you|error|\[|Approve|Deny/i.test(frame), "status only: no decision, button or error text")
    assert.ok(line("reader").indexOf("reader") > line("researcher").indexOf("researcher"), `sub-sub-agents are indented\n${frame}`)
    controller.moveFocus(1)
    await harness.renderOnce()
    assert.equal(controller.focusedPanelId, "writer")
    assert.equal(harness.captureCharFrame().split("\n").filter(row => row.includes("writer")).length, 1, "moving the focus does not duplicate or lose a panel")
  } finally { harness.renderer.destroy() }
})
