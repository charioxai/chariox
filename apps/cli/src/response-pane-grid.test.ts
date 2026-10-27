import assert from "node:assert/strict"
import test from "node:test"

import { buildPaneGridModel } from "./response-pane-grid.js"

const agents = [
  { id: "agent-a" },
  { id: "agent-b" },
  { id: "agent-c" },
]

test("pane grid gives two visible panes one shared vertical seam", () => {
  const model = buildPaneGridModel({
    paneRows: [[0, 1]],
    visibleAgents: agents,
    focusedAgentId: "agent-a",
    split: true,
    showWorkflowScreen: false,
  })

  assert.deepEqual(model.rows[0]?.verticals.map((segment) => segment.visible), [true, true, true])
  assert.deepEqual(model.rows[0]?.verticals.map((segment) => segment.tone), ["focused", "focused", "subtle"])
  assert.deepEqual(model.borderRows[0]?.junctions.map((junction) => junction.char), ["┌", "┬", "┐"])
  assert.deepEqual(model.borderRows[1]?.junctions.map((junction) => junction.char), ["└", "┴", "┘"])
})

test("pane grid lets a single lower pane span both columns", () => {
  const model = buildPaneGridModel({
    paneRows: [[0, 1], [2]],
    visibleAgents: agents,
    focusedAgentId: "agent-c",
    split: true,
    showWorkflowScreen: false,
  })

  assert.deepEqual(model.rows[1]?.slots, [{
    paneIndex: 2,
    agentId: "agent-c",
    visible: true,
    focused: true,
    colStart: 0,
    colSpan: 2,
  }])
  assert.deepEqual(model.rows[1]?.verticals.map((segment) => segment.visible), [true, false, true])
  assert.deepEqual(model.rows[1]?.verticals.map((segment) => segment.tone), ["focused", "subtle", "focused"])
  assert.deepEqual(model.borderRows[1]?.junctions.map((junction) => junction.char), ["├", "┴", "┤"])
})

test("pane grid hides auxiliary panes on workflow screen without orphan borders", () => {
  const model = buildPaneGridModel({
    paneRows: [[0, 1], [2]],
    visibleAgents: agents,
    focusedAgentId: "agent-a",
    split: true,
    showWorkflowScreen: true,
  })

  assert.equal(model.rows[0]?.visible, true)
  assert.equal(model.rows[1]?.visible, false)
  assert.deepEqual(model.rows[0]?.slots.map((slot) => slot.paneIndex), [0])
  assert.deepEqual(model.rows[0]?.verticals.map((segment) => segment.visible), [true, false, true])
  assert.equal(model.borderRows[0]?.visible, true)
  assert.equal(model.borderRows[1]?.visible, true)
  assert.equal(model.borderRows[2]?.visible, false)
})

test("pane grid preserves the single left rail outside split mode", () => {
  const model = buildPaneGridModel({
    paneRows: [[0, 1], [2]],
    visibleAgents: agents,
    focusedAgentId: "agent-a",
    split: false,
    showWorkflowScreen: false,
  })

  assert.deepEqual(model.rows[0]?.verticals.map((segment) => segment.visible), [true, false, false])
  assert.equal(model.borderRows[0]?.visible, false)
  assert.equal(model.borderRows[1]?.visible, false)
  assert.equal(model.rows[1]?.visible, false)
})
