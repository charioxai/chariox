import assert from "node:assert/strict"
import test from "node:test"

import {
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

test("formatSplitPaneFooter includes alias and runtime config metadata", () => {
  assert.equal(
    formatSplitPaneFooter(primaryAgent, null, null),
    "Planner • OpenAI • GPT-5.4",
  )
  assert.equal(
    formatSplitPaneFooter(secondaryAgent, null, "gpt-5.4"),
    "agent • OpenAI • GPT-5.4",
  )
})

test("formatSplitPaneFooter prefers the active run model for the matching agent", () => {
  const activeRun: SplitPaneFooterActiveRun = {
    agentInstanceId: "agent-a",
    provider: "opencode",
    model: "opencode/gpt-5.4",
    variant: "high",
  }

  assert.equal(
    formatSplitPaneFooter(
      { ...primaryAgent, provider: "opencode", model: "opencode/gpt-5.3" },
      activeRun,
      null,
    ),
    "Planner • OpenCode • OpenCode GPT-5.4 • High",
  )
})

test("formatSplitPaneFooter prefers an override variant when idle", () => {
  assert.equal(
    formatSplitPaneFooter(primaryAgent, null, null, { variant: "high" }),
    "Planner • OpenAI • GPT-5.4 • High",
  )
})

test("formatSplitPaneFooter marks Meta mode agents", () => {
  assert.equal(
    formatSplitPaneFooter({ ...primaryAgent, meta_mode: { activated_at_ms: 1 } }, null, null),
    "Planner • Meta mode • OpenAI • GPT-5.4",
  )
})

test("formatSplitPaneFooterParts includes agent identity and provider metadata", () => {
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
    ],
  )
})

test("formatSplitPaneFooterParts shows only view slice before the menu for slice agents", () => {
  const parts = formatSplitPaneFooterParts({ ...primaryAgent, location_label: "slice:dev" }, null, null)
  assert.deepEqual(
    parts.map((part) => ({
      kind: part.kind,
      text: part.text,
    })),
    [
      { kind: "agent", text: "Planner" },
      { kind: "location", text: "view slice" },
      { kind: "provider", text: "OpenAI" },
      { kind: "model", text: "GPT-5.4" },
    ],
  )
})

test("formatSplitPaneFooterParts includes mode and permission overrides", () => {
  const parts = formatSplitPaneFooterParts({
    ...primaryAgent,
    execution_mode: "plan",
    permission_level: "required",
  }, null, null)
  assert.deepEqual(
    parts.map((part) => ({
      kind: part.kind,
      text: part.text,
    })),
    [
      { kind: "agent", text: "Planner" },
      { kind: "provider", text: "OpenAI" },
      { kind: "model", text: "GPT-5.4" },
      { kind: "mode", text: "plan" },
      { kind: "permission", text: "required" },
    ],
  )
})

test("formatSplitPaneFooterParts shows active substitute summary", () => {
  const parts = formatSplitPaneFooterParts({
    ...primaryAgent,
    substitutes: [
      { provider: "codex", model: "gpt-5.4", variant: "high" },
    ],
    active_substitute_index: 0,
  }, null, null)
  assert.deepEqual(
    parts.map((part) => ({
      kind: part.kind,
      text: part.text,
    })),
    [
      { kind: "agent", text: "Planner" },
      { kind: "provider", text: "OpenAI" },
      { kind: "model", text: "GPT-5.4" },
      { kind: "substitute", text: "sub 1: codex/gpt-5.4/high" },
    ],
  )
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
  assert.equal(state.secondary.info, "agent • OpenAI • GPT-5.4")
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
      provider: "opencode",
      model: "opencode/gpt-5.4",
      variant: "high",
    },
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "IDLE", tone: "idle" })
  assert.deepEqual(state.secondary.badge, { label: "READING", tone: "working" })
  assert.equal(state.primary.focused, true)
  assert.equal(state.primary.info, "Planner • OpenCode • OpenCode GPT-5.4 • High")
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

test("buildSplitPaneFooterState can treat projected activity as authoritative over agent state", () => {
  const state = buildSplitPaneFooterState({
    mode: "idle",
    selection: {
      primary: { ...primaryAgent, state: "Working", is_processing: true },
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
    useLegacyAgentProcessingState: false,
    activeRun: null,
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "IDLE", tone: "idle" })
})
