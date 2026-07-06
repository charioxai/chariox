import assert from "node:assert/strict"
import test from "node:test"

import {
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
}

const secondaryAgent: SplitPaneFooterAgent = {
  id: "agent-b",
  agent_ref: "agent-b",
  alias: null,
  provider: "openai",
  model: null,
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
