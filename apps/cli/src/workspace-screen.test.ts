import assert from "node:assert/strict"
import test from "node:test"

import {
  resolveWorkspaceVisibleAgents,
  resolveWorkspaceVisibleTranscriptAgentId,
  toggleWorkspaceScreenMode,
} from "./workspace-screen.js"

test("toggleWorkspaceScreenMode flips between agents and workflow", () => {
  assert.equal(toggleWorkspaceScreenMode("agents"), "workflow")
  assert.equal(toggleWorkspaceScreenMode("workflow"), "agents")
})

test("workflow screen hides panes without mutating the focused agent binding", () => {
  const visibleAgents = [{ id: "agent-a" }, { id: "agent-b" }]

  assert.deepEqual(resolveWorkspaceVisibleAgents("workflow", visibleAgents), [])
  assert.equal(resolveWorkspaceVisibleTranscriptAgentId("workflow", "agent-b"), null)

  assert.deepEqual(resolveWorkspaceVisibleAgents("agents", visibleAgents), visibleAgents)
  assert.equal(resolveWorkspaceVisibleTranscriptAgentId("agents", "agent-b"), "agent-b")
})
