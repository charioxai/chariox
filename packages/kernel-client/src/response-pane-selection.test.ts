import assert from "node:assert/strict"
import test from "node:test"

import {
  responsePaneBindingsMatch,
  selectResponsePaneAgents,
  splitPaneAuxiliaryAgentIds,
} from "./response-pane-selection.js"

const agents = [
  { id: "agent-a" },
  { id: "agent-b" },
  { id: "agent-c" },
  { id: "agent-d" },
]

test("selectResponsePaneAgents uses the focused agent outside split mode", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "agent-c", false), {
    visibleAgents: [agents[2]],
    primary: agents[2],
    secondary: null,
    tertiary: null,
    visibleTranscriptAgentId: "agent-c",
    screenIndex: 0,
    screenCount: 1,
  })
})

test("selectResponsePaneAgents falls back to the first agent when focus is missing", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "missing", false), {
    visibleAgents: [agents[0]],
    primary: agents[0],
    secondary: null,
    tertiary: null,
    visibleTranscriptAgentId: "agent-a",
    screenIndex: 0,
    screenCount: 1,
  })
})

test("selectResponsePaneAgents keeps split panes bound to the first three agents", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "agent-c", true), {
    visibleAgents: [agents[0], agents[1], agents[2]],
    primary: agents[0],
    secondary: agents[1],
    tertiary: agents[2],
    visibleTranscriptAgentId: "agent-a",
    screenIndex: 0,
    screenCount: 2,
  })
})

test("selectResponsePaneAgents falls back to the first agent in split mode when focus is missing", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "missing", true), {
    visibleAgents: [agents[0], agents[1], agents[2]],
    primary: agents[0],
    secondary: agents[1],
    tertiary: agents[2],
    visibleTranscriptAgentId: "agent-a",
    screenIndex: 0,
    screenCount: 2,
  })
})

test("splitPaneAuxiliaryAgentIds returns only the auxiliary split panes", () => {
  assert.deepEqual(splitPaneAuxiliaryAgentIds(agents, "agent-c", true), ["agent-b", "agent-c"])
  assert.deepEqual(splitPaneAuxiliaryAgentIds(agents, "agent-a", true), ["agent-b", "agent-c"])
  assert.deepEqual(splitPaneAuxiliaryAgentIds(agents, "agent-c", false), [])
})

test("selectResponsePaneAgents pages panes by focused agent and max agents per screen", () => {
  assert.deepEqual(selectResponsePaneAgents(agents, "agent-d", true, 2), {
    visibleAgents: [agents[2], agents[3]],
    primary: agents[2],
    secondary: agents[3],
    tertiary: null,
    visibleTranscriptAgentId: "agent-c",
    screenIndex: 1,
    screenCount: 2,
  })
})

test("responsePaneBindingsMatch ignores focus changes within the same split screen", () => {
  const left = selectResponsePaneAgents(agents, "agent-a", true, 3)
  const right = selectResponsePaneAgents(agents, "agent-c", true, 3)
  const nextScreen = selectResponsePaneAgents(agents, "agent-d", true, 2)

  assert.equal(responsePaneBindingsMatch(left, right), true)
  assert.equal(responsePaneBindingsMatch(left, nextScreen), false)
})
