import assert from "node:assert/strict"
import test from "node:test"

import {
  agentPaneStatusBadge,
  buildSplitPaneFooterState,
  formatSplitPaneFooter,
  formatSplitPaneFooterParts,
  reflectedDistance,
  type SplitPaneFooterActiveRun,
  type SplitPaneFooterAgent,
} from "./split-pane-footer.js"

const primaryAgent: SplitPaneFooterAgent = {
  id: "agent-a",
  agent_ref: "agent-a",
  alias: "Planner",
  provider: "openai",
  model: "gpt-5.4",
  state: "Idle" as const,
  is_processing: false,
}

const secondaryAgent: SplitPaneFooterAgent = {
  id: "agent-b",
  agent_ref: "agent-b",
  alias: null,
  provider: "openai",
  model: null,
  state: "Working" as const,
  is_processing: false,
}

test("reflectedDistance bounces the highlight across the badge width", () => {
  assert.equal(reflectedDistance(0, 5, 0), 0)
  assert.equal(reflectedDistance(4, 5, 4), 0)
  assert.equal(reflectedDistance(3, 5, 5), 0)
})

test("agentPaneStatusBadge prefers explicit activity labels", () => {
  assert.deepEqual(agentPaneStatusBadge(primaryAgent, "patching"), {
    label: "PATCHING",
    tone: "working",
  })
})

test("agentPaneStatusBadge reports error and streaming states", () => {
  assert.deepEqual(agentPaneStatusBadge({ ...primaryAgent, state: "Error" }, null), {
    label: "ERROR",
    tone: "error",
  })
  assert.deepEqual(agentPaneStatusBadge(primaryAgent, null, false, true), {
    label: "THINKING",
    tone: "working",
  })
})

test("agentPaneStatusBadge stays working while the agent still has prompt work", () => {
  assert.deepEqual(agentPaneStatusBadge(primaryAgent, null, true, false), {
    label: "THINKING",
    tone: "working",
  })
})

test("agentPaneStatusBadge stays working while the local busy latch is active", () => {
  assert.deepEqual(agentPaneStatusBadge(primaryAgent, null, false, false, true), {
    label: "THINKING",
    tone: "working",
  })
})

test("formatSplitPaneFooter uses alias and prompt-style model metadata", () => {
  assert.equal(
    formatSplitPaneFooter(primaryAgent, null, null),
    "Planner • OpenAI • GPT-5.4 • build • yolo",
  )
  assert.equal(
    formatSplitPaneFooter(secondaryAgent, null, "gpt-5.4"),
    "agent-b • OpenAI • GPT-5.4 • build • yolo",
  )
})

test("formatSplitPaneFooter prefers the active run model for the matching agent", () => {
  const activeRun: SplitPaneFooterActiveRun = {
    agentInstanceId: "agent-a",
    model: "openai/gpt-5.4",
    variant: "high",
  }

  assert.equal(
    formatSplitPaneFooter(
      { ...primaryAgent, provider: "opencode", model: "openai/gpt-5.3-codex-spark" },
      activeRun,
      null,
    ),
    "Planner • OpenAI • GPT-5.4 • High • build • yolo",
  )
})

test("formatSplitPaneFooter prefers an override variant when idle", () => {
  assert.equal(
    formatSplitPaneFooter(primaryAgent, null, null, { variant: "high" }),
    "Planner • OpenAI • GPT-5.4 • High • build • yolo",
  )
})

test("formatSplitPaneFooterParts mirrors the prompt footer order with an agent prefix", () => {
  const parts = formatSplitPaneFooterParts(primaryAgent, null, null)
  assert.deepEqual(
    parts.map((part) => ({
      kind: part.kind,
      text: part.text,
    })),
    [
      { kind: "agent", text: "Planner" },
      { kind: "provider", text: "OpenAI" },
      { kind: "model", text: "GPT-5.4" },
      { kind: "mode", text: "build" },
      { kind: "permission", text: "yolo" },
    ],
  )
  assert.equal(parts[1]?.tone, "info")
  assert.equal(parts[2]?.tone, "secondary")
})

test("buildSplitPaneFooterState keeps disconnected panes uniformly disconnected", () => {
  const state = buildSplitPaneFooterState({
    mode: "disconnected",
    selection: {
      primary: primaryAgent,
      secondary: secondaryAgent,
      tertiary: null,
    },
    focusedAgentId: "agent-b",
    streamingAgentId: "agent-b",
    activityLabels: {
      "agent-a": "thinking",
      "agent-b": "patching",
    },
    activeRun: null,
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "DISCONNECTED", tone: "disconnected" })
  assert.deepEqual(state.secondary.badge, { label: "DISCONNECTED", tone: "disconnected" })
  assert.equal(state.secondary.focused, true)
  assert.equal(state.secondary.info, "agent-b • OpenAI • GPT-5.4 • build • yolo")
})

test("buildSplitPaneFooterState uses activity labels and focus per pane", () => {
  const state = buildSplitPaneFooterState({
    mode: "working",
    selection: {
      primary: primaryAgent,
      secondary: secondaryAgent,
      tertiary: null,
    },
    focusedAgentId: "agent-a",
    streamingAgentId: "agent-b",
    activityLabels: {
      "agent-a": null,
      "agent-b": "reading",
    },
    hasPromptWorkByAgent: {
      "agent-a": false,
      "agent-b": true,
    },
    activeRun: {
      agentInstanceId: "agent-a",
      model: "openai/gpt-5.4",
      variant: "high",
    },
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "IDLE", tone: "idle" })
  assert.deepEqual(state.secondary.badge, { label: "READING", tone: "working" })
  assert.equal(state.primary.focused, true)
  assert.equal(state.primary.info, "Planner • OpenAI • GPT-5.4 • High • build • yolo")
})

test("buildSplitPaneFooterState keeps a pane working while its busy latch is set", () => {
  const state = buildSplitPaneFooterState({
    mode: "working",
    selection: {
      primary: primaryAgent,
      secondary: secondaryAgent,
      tertiary: null,
    },
    focusedAgentId: "agent-a",
    streamingAgentId: null,
    activityLabels: {
      "agent-a": null,
      "agent-b": null,
    },
    hasPromptWorkByAgent: {
      "agent-a": false,
      "agent-b": false,
    },
    busyLatchesByAgent: {
      "agent-a": true,
    },
    activeRun: null,
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "THINKING", tone: "working" })
})

test("buildSplitPaneFooterState marks an agent working for the turn and idle after completion", () => {
  const workingState = buildSplitPaneFooterState({
    mode: "working",
    selection: {
      primary: primaryAgent,
      secondary: null,
      tertiary: null,
    },
    focusedAgentId: "agent-a",
    streamingAgentId: null,
    activityLabels: {
      "agent-a": null,
    },
    hasPromptWorkByAgent: {
      "agent-a": true,
    },
    activeRun: null,
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(workingState.primary.badge, { label: "THINKING", tone: "working" })

  const completedState = buildSplitPaneFooterState({
    mode: "idle",
    selection: {
      primary: primaryAgent,
      secondary: null,
      tertiary: null,
    },
    focusedAgentId: "agent-a",
    streamingAgentId: null,
    activityLabels: {
      "agent-a": null,
    },
    hasPromptWorkByAgent: {
      "agent-a": false,
    },
    busyLatchesByAgent: {
      "agent-a": false,
    },
    activeRun: null,
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(completedState.primary.badge, { label: "IDLE", tone: "idle" })
})
