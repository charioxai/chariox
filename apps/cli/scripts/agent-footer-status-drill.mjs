import assert from "node:assert/strict"

import {
  sessionAgentPaneStatusBadge,
} from "@chariox/kernel-client/session-runtime-status"

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
  return sessionAgentPaneStatusBadge({
    agent: options.agent,
    activeLabel: options.activityLabel ?? null,
    hasPromptWork: options.hasPromptWork,
    isStreaming: options.streaming ?? false,
    busyLatch: options.busyLatch ?? false,
  })
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
