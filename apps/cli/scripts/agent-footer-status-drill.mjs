import assert from "node:assert/strict"

import {
  buildSplitPaneFooterState,
} from "../dist/split-pane-footer.js"

const agent = {
  id: "agent-a",
  agent_ref: "agent-a",
  alias: "native-a",
  provider: "codex",
  model: "gpt-5.4",
  effort: "high",
  state: "Idle",
  is_processing: false,
}

function badgeFor(options) {
  return buildSplitPaneFooterState({
    mode: options.mode,
    selection: {
      primary: options.agent,
      secondary: null,
      tertiary: null,
    },
    focusedAgentId: options.agent.id,
    streamingAgentId: options.streaming ? options.agent.id : null,
    activityLabels: {
      [options.agent.id]: options.activityLabel ?? null,
    },
    hasPromptWorkByAgent: {
      [options.agent.id]: options.hasPromptWork,
    },
    busyLatchesByAgent: {
      [options.agent.id]: options.busyLatch ?? false,
    },
    activeRun: null,
    fallbackModel: "gpt-5.4",
  }).primary.badge
}

const beginningOfTurn = badgeFor({
  mode: "working",
  agent,
  hasPromptWork: true,
})
assert.deepEqual(beginningOfTurn, { label: "THINKING", tone: "working" })

const streamingTurn = badgeFor({
  mode: "working",
  agent,
  hasPromptWork: false,
  streaming: true,
})
assert.deepEqual(streamingTurn, { label: "THINKING", tone: "working" })

const afterSummary = badgeFor({
  mode: "idle",
  agent: {
    ...agent,
    state: "Idle",
    is_processing: false,
  },
  hasPromptWork: false,
})
assert.deepEqual(afterSummary, { label: "IDLE", tone: "idle" })

console.log(JSON.stringify({
  status: "ok",
  drill: "agent-footer-status",
  beginningOfTurn,
  streamingTurn,
  afterSummary,
}, null, 2))
