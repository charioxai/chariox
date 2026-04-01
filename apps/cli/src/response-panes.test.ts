import assert from "node:assert/strict"
import test from "node:test"

import { computeSplitPaneGeometry, selectResponsePaneAgents, splitPaneAuxiliaryAgentIds } from "./response-panes.js"

const agents = [
  { id: "agent-a" },
  { id: "agent-b" },
  { id: "agent-c" },
  { id: "agent-d" },
]

test("selectResponsePaneAgents uses the focused agent outside split mode", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "agent-c", false), {
    primary: agents[2],
    secondary: null,
    tertiary: null,
    visibleTranscriptAgentId: "agent-c",
  })
})

test("selectResponsePaneAgents falls back to the first agent when focus is missing", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "missing", false), {
    primary: agents[0],
    secondary: null,
    tertiary: null,
    visibleTranscriptAgentId: "agent-a",
  })
})

test("selectResponsePaneAgents uses the first three agents in split mode", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "agent-c", true), {
    primary: agents[2],
    secondary: agents[0],
    tertiary: agents[1],
    visibleTranscriptAgentId: "agent-c",
  })
})

test("selectResponsePaneAgents falls back to the first agent in split mode when focus is missing", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "missing", true), {
    primary: agents[0],
    secondary: agents[1],
    tertiary: agents[2],
    visibleTranscriptAgentId: "agent-a",
  })
})

test("splitPaneAuxiliaryAgentIds returns only the auxiliary split panes", () => {
  assert.deepEqual(splitPaneAuxiliaryAgentIds(agents, "agent-c", true), ["agent-a", "agent-b"])
  assert.deepEqual(splitPaneAuxiliaryAgentIds(agents, "agent-a", true), ["agent-b", "agent-c"])
  assert.deepEqual(splitPaneAuxiliaryAgentIds(agents, "agent-c", false), [])
})

test("computeSplitPaneGeometry preserves the current two-column split behavior", () => {
  assert.deepEqual(computeSplitPaneGeometry(120, true, true, false), {
    showSecondaryPane: true,
    showTertiaryPane: false,
    splitPaneWidth: 56,
    layoutDirection: "row",
    layoutGap: 1,
    topRowVisible: true,
    topRowGap: 1,
    topRowFlexBasis: "auto",
    topRowMinHeight: null,
    primaryFlexGrow: 0,
    primaryWidth: 56,
    primaryFlexBasis: 56,
    primaryMinWidth: 56,
    primaryMaxWidth: 56,
    secondaryWidth: 56,
    secondaryFlexBasis: 56,
    secondaryMinWidth: 56,
    secondaryMaxWidth: 56,
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
    splitPaneWidth: 56,
    layoutDirection: "column",
    layoutGap: 1,
    topRowVisible: true,
    topRowGap: 1,
    topRowFlexBasis: 0,
    topRowMinHeight: 0,
    primaryFlexGrow: 0,
    primaryWidth: 56,
    primaryFlexBasis: 56,
    primaryMinWidth: 56,
    primaryMaxWidth: 56,
    secondaryWidth: 56,
    secondaryFlexBasis: 56,
    secondaryMinWidth: 56,
    secondaryMaxWidth: 56,
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
    splitPaneWidth: 56,
    layoutDirection: "row",
    layoutGap: 0,
    topRowVisible: true,
    topRowGap: 0,
    topRowFlexBasis: "auto",
    topRowMinHeight: null,
    primaryFlexGrow: 1,
    primaryWidth: 112,
    primaryFlexBasis: 112,
    primaryMinWidth: 112,
    primaryMaxWidth: 112,
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
