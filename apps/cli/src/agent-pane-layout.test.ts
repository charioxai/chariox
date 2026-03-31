import assert from "node:assert/strict"
import test from "node:test"

import { buildAgentPaneRows } from "./agent-pane-layout.js"

test("buildAgentPaneRows keeps one full-width pane for one agent", () => {
  assert.deepEqual(buildAgentPaneRows(1), [{ key: "row-1", slots: [0] }])
})

test("buildAgentPaneRows uses a vertical split for two agents", () => {
  assert.deepEqual(buildAgentPaneRows(2), [{ key: "row-1", slots: [0, 1] }])
})

test("buildAgentPaneRows gives the third agent a full-width lower pane", () => {
  assert.deepEqual(buildAgentPaneRows(3), [
    { key: "row-1", slots: [0, 1] },
    { key: "row-2", slots: [2] },
  ])
})

test("buildAgentPaneRows keeps a lower row split for four agents", () => {
  assert.deepEqual(buildAgentPaneRows(4), [
    { key: "row-1", slots: [0, 1] },
    { key: "row-2", slots: [2, 3] },
  ])
})

test("buildAgentPaneRows adds a final vertical split for agents five and six", () => {
  assert.deepEqual(buildAgentPaneRows(5), [
    { key: "row-1", slots: [0, 1, 2] },
    { key: "row-2", slots: [3, 4, null] },
  ])
  assert.deepEqual(buildAgentPaneRows(6), [
    { key: "row-1", slots: [0, 1, 2] },
    { key: "row-2", slots: [3, 4, 5] },
  ])
})
