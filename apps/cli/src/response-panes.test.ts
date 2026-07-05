import assert from "node:assert/strict"
import test from "node:test"

import {
  computeSplitPaneGeometry,
  responsePaneRowSlots,
  workflowCanvasPaneIndices,
} from "./response-panes.js"

test("responsePaneRowSlots lays out two panes per row up to the configured screen size", () => {
  assert.deepEqual(responsePaneRowSlots(1), [[0]])
  assert.deepEqual(responsePaneRowSlots(4), [[0, 1], [2, 3]])
  assert.deepEqual(responsePaneRowSlots(5), [[0, 1], [2, 3], [4]])
})

test("workflowCanvasPaneIndices exposes spare trailing split-pane slots for the workflow canvas", () => {
  assert.deepEqual(
    workflowCanvasPaneIndices({
      split: true,
      visibleAgentCount: 1,
      screenIndex: 1,
      screenCount: 2,
      maxAgentsPerScreen: 3,
    }),
    [1, 2],
  )
  assert.deepEqual(
    workflowCanvasPaneIndices({
      split: true,
      visibleAgentCount: 3,
      screenIndex: 0,
      screenCount: 2,
      maxAgentsPerScreen: 3,
    }),
    [],
  )
  assert.deepEqual(
    workflowCanvasPaneIndices({
      split: false,
      visibleAgentCount: 1,
      screenIndex: 0,
      screenCount: 1,
      maxAgentsPerScreen: 3,
    }),
    [],
  )
})

test("computeSplitPaneGeometry uses the full width with no split-pane gaps", () => {
  assert.deepEqual(computeSplitPaneGeometry(120, true, true, false), {
    showSecondaryPane: true,
    showTertiaryPane: false,
    splitPaneWidth: 60,
    layoutDirection: "row",
    layoutGap: 0,
    topRowVisible: true,
    topRowGap: 0,
    topRowFlexBasis: "auto",
    topRowMinHeight: null,
    primaryFlexGrow: 0,
    primaryWidth: 60,
    primaryFlexBasis: 60,
    primaryMinWidth: 60,
    primaryMaxWidth: 60,
    secondaryWidth: 60,
    secondaryFlexBasis: 60,
    secondaryMinWidth: 60,
    secondaryMaxWidth: 60,
    tertiaryWidth: 0,
    tertiaryFlexGrow: 0,
    tertiaryFlexBasis: 0,
    tertiaryMinHeight: 0,
  })
})

test("computeSplitPaneGeometry stacks the tertiary pane below the top row", () => {
  assert.deepEqual(computeSplitPaneGeometry(120, true, true, true), {
    showSecondaryPane: true,
    showTertiaryPane: true,
    splitPaneWidth: 60,
    layoutDirection: "column",
    layoutGap: 0,
    topRowVisible: true,
    topRowGap: 0,
    topRowFlexBasis: 0,
    topRowMinHeight: 0,
    primaryFlexGrow: 0,
    primaryWidth: 60,
    primaryFlexBasis: 60,
    primaryMinWidth: 60,
    primaryMaxWidth: 60,
    secondaryWidth: 60,
    secondaryFlexBasis: 60,
    secondaryMinWidth: 60,
    secondaryMaxWidth: 60,
    tertiaryWidth: "auto",
    tertiaryFlexGrow: 1,
    tertiaryFlexBasis: 0,
    tertiaryMinHeight: 0,
  })
})

test("computeSplitPaneGeometry keeps the primary pane visible in split mode without auxiliaries", () => {
  assert.deepEqual(computeSplitPaneGeometry(120, true, false, false), {
    showSecondaryPane: false,
    showTertiaryPane: false,
    splitPaneWidth: 60,
    layoutDirection: "row",
    layoutGap: 0,
    topRowVisible: true,
    topRowGap: 0,
    topRowFlexBasis: "auto",
    topRowMinHeight: null,
    primaryFlexGrow: 1,
    primaryWidth: 120,
    primaryFlexBasis: 120,
    primaryMinWidth: 120,
    primaryMaxWidth: 120,
    secondaryWidth: 0,
    secondaryFlexBasis: 0,
    secondaryMinWidth: 0,
    secondaryMaxWidth: 0,
    tertiaryWidth: 0,
    tertiaryFlexGrow: 0,
    tertiaryFlexBasis: 0,
    tertiaryMinHeight: 0,
  })
})
