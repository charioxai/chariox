import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import {
  agentPaneStatusBadge,
  buildSplitPaneFooterState,
  formatSplitPaneFooter,
  reflectedDistance,
  type SplitPaneFooterActiveRun,
  type SplitPaneFooterAgent,
} from "./split-pane-footer.js"

const catalog = fallbackProviderCatalog()

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
  assert.deepEqual(agentPaneStatusBadge(primaryAgent, null, true), {
    label: "WORKING",
    tone: "working",
  })
})

test("formatSplitPaneFooter uses alias and catalog model name", () => {
  assert.equal(
    formatSplitPaneFooter(primaryAgent, catalog, null, null),
    "Planner • GPT-5.4",
  )
  assert.equal(
    formatSplitPaneFooter(secondaryAgent, catalog, null, "gpt-5.4"),
    "agent-b • GPT-5.4",
  )
})

test("formatSplitPaneFooter prefers the active run model for the matching agent", () => {
  const activeRun: SplitPaneFooterActiveRun = {
    agentInstanceId: "agent-a",
    model: "openai/gpt-5.4",
  }

  assert.equal(
    formatSplitPaneFooter(
      { ...primaryAgent, provider: "opencode", model: "openai/gpt-5.3-codex-spark" },
      catalog,
      activeRun,
      null,
    ),
    "Planner • GPT-5.4",
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
    catalog,
    activeRun: null,
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "DISCONNECTED", tone: "disconnected" })
  assert.deepEqual(state.secondary.badge, { label: "DISCONNECTED", tone: "disconnected" })
  assert.equal(state.secondary.focused, true)
  assert.equal(state.secondary.info, "agent-b • GPT-5.4")
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
    catalog,
    activeRun: {
      agentInstanceId: "agent-a",
      model: "openai/gpt-5.4",
    },
    fallbackModel: "gpt-5.4",
  })

  assert.deepEqual(state.primary.badge, { label: "IDLE", tone: "idle" })
  assert.deepEqual(state.secondary.badge, { label: "READING", tone: "working" })
  assert.equal(state.primary.focused, true)
  assert.equal(state.primary.info, "Planner • GPT-5.4")
})
