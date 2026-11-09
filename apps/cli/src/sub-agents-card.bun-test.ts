// MP-08 / MP-09 / MP-10 / MP-11 A6 (TUI): the list card is the dialog popup with the hierarchy as rows.
import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { AgentTree, AgentTreeNode, AgentTreeState } from "@chariox/kernel-client/kernel-types"
import { subAgentRows, visibleSubAgentRows } from "@chariox/kernel-client/sub-agent-list"
import { renderCliDialogOverlay } from "./cli-dialog-overlay.js"
import { dialogTextWidth } from "./cli-dialog-panel-size.js"
import { subAgentsCardRows } from "./sub-agents-card-view.js"

const node = (id: string, parent: string, state: AgentTreeState, n: number, needs_user = false): AgentTreeNode => ({
  agent_id: id, parent_agent_id: parent, kernel_id: null, alias: id, provider: "p", model: null, depth: 1, state, outcome: null, deadline_ms: null, created_at_ms: n, needs_user,
})
const tree: AgentTree = {
  root: "room", rev: 1, totals: { live: 4, running: 2, needs_user: 1 },
  nodes: [node("researcher", "room", "running", 1), node("reader", "researcher", "running", 2), node("checker", "reader", "waiting", 3, true), node("writer", "room", "ended", 4)],
}

test("AO-UI-02 rows indent under their parent at every depth, one state word each, finished dimmed, the selected row focused", () => {
  const lines = visibleSubAgentRows(subAgentRows(tree), new Set())
  const rows = subAgentsCardRows(lines, 1, new Set(), 60)
  assert.deepEqual(rows.map(row => row.text.indexOf(row.text.trim().split(" ").find(part => /^[a-z]/.test(part))!)), [2, 4, 6, 2], "two columns of indent per depth, then the twisty")
  assert.deepEqual(rows.map(row => row.text.split(" ").filter(Boolean).slice(-1).concat(row.text.includes("needs you") ? ["needs you"] : [])[0]), ["running", "running", "you", "done"])
  assert.deepEqual(rows.map(row => row.text.trimStart()[0]), ["▾", "▾", "c", "w"])
  assert.deepEqual(rows.map(row => [row.tone, row.focused]), [["text", false], ["text", true], ["warning", false], ["muted", false]])
  assert.ok(rows.every(row => row.text.length === 60), "the state word is aligned to the right edge")
  const collapsed = subAgentsCardRows(visibleSubAgentRows(subAgentRows(tree), new Set(["researcher"])), 0, new Set(["researcher"]), 60)
  assert.deepEqual(collapsed.map(row => row.text.slice(0, 12).trim()), ["▸ researcher", "writer"])
})

test("AO-UI-02 a long name clips before the state word", () => {
  const long: AgentTree = { ...tree, nodes: [node("x".repeat(80), "room", "running", 1)] }
  const [row] = subAgentsCardRows(visibleSubAgentRows(subAgentRows(long), new Set()), 0, new Set(), 40)
  assert.equal(row!.text.length, 40); assert.ok(row!.text.endsWith(" running"))
})

for (const width of [80, 140]) test(`AO-UI-02 the card is the dialog popup at ${width} columns: rows only, no title`, async () => {
  const height = 24
  const harness = await createTestRenderer({ width, height, useThread: false })
  const overlayBox = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0, width, height })
  harness.renderer.root.add(overlayBox)
  try {
    const rows = subAgentsCardRows(visibleSubAgentRows(subAgentRows(tree), new Set()), 0, new Set(), dialogTextWidth(width))
    renderCliDialogOverlay({ overlayBox, renderer: harness.renderer, dimensions: { width, height }, mode: "sub-agents", onDismiss() {}, sessions: [], normalizeSessionBrowserIndex: () => 0, terminalPairing: null, terminalPairingQrLines: [], hotkeySections: [], subAgentsRows: rows })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    for (const text of ["researcher", "reader", "checker", "needs you", "writer", "done"]) assert.ok(frame.includes(text), `${text}\n${frame}`)
    assert.ok(!/Esc|close|error/i.test(frame), "no title, hint or error text")
  } finally { harness.renderer.destroy() }
})
