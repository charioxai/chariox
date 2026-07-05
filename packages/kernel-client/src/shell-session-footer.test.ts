import assert from "node:assert/strict"
import test from "node:test"

import type { WorkspaceLiveSyncStatus } from "./kernel-types.js"
import {
  sessionAttachedFooterSummary,
  sessionFooterHint,
  sessionVisibleAgentSummary,
} from "./shell-session-footer.js"
import { makeAgent, makeSession } from "./shell-executor.test-support.js"

test("session footer hint reflects errors, active prompts, queues, and fallback status", () => {
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: "prompt-1",
    queueDepth: 2,
    statusLine: "Connected.",
  }), "Processing prompt-1; 2 queued.")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: "prompt-1",
    queueDepth: 0,
    statusLine: "Connected.",
  }), "Processing prompt-1.")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: null,
    queueDepth: 1,
    statusLine: "Connected.",
  }), "1 queued prompt.")
  assert.equal(sessionFooterHint({
    fatalError: "boom",
    activePromptId: "prompt-1",
    queueDepth: 0,
    statusLine: "Connected.",
  }), "boom")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: null,
    queueDepth: 0,
    statusLine: "Connected.",
  }), "Connected.")
})

test("session attached footer summary projects visible agents, collaboration, and sync", () => {
  assert.equal(sessionAttachedFooterSummary({
    session: makeSession({
      alias: "feature-refactor",
      agents: [
        makeAgent({ id: "agent-a", agent_ref: "main" }),
        makeAgent({ id: "agent-b", agent_ref: "review", alias: "QA", is_processing: true }),
      ],
    }),
    connectedClientCount: 2,
    multiAgentMode: true,
    sessionStatusMode: "working",
    hotkeyToggleLabel: "Ctrl+T",
  }), "Session feature-refactor • 2 CLIs connected • 2 visible agents • Ctrl+C to stop • Tab cycles focus • Ctrl+P opens workflow • Ctrl+T hotkeys")

  const sharedSession = makeSession({
    alias: "shared-review",
    agents: [makeAgent({ id: "agent-a" })],
    collaboration_agent_counts: {
      owned_agent_count: 1,
      other_user_agent_count: 3,
      total_agent_count: 4,
      collaborator_count: 2,
    },
  })
  const sharedSummary = sessionAttachedFooterSummary({
    session: sharedSession,
    connectedClientCount: 3,
    multiAgentMode: false,
    sessionStatusMode: "idle",
    hotkeyToggleLabel: "Ctrl+T",
  })
  assert.equal(sharedSummary, "Session shared-review • 3 CLIs connected • 1 visible agent • 3 collaborator agents • 2 collaborators • Ctrl+T hotkeys")
  assert.equal(sessionVisibleAgentSummary(sharedSession), "1 visible agent • 3 collaborator agents • 2 collaborators")
  assert.doesNotMatch(sharedSummary, /user-|agent-a|owner/)

  assert.equal(sessionAttachedFooterSummary({
    session: makeSession({ alias: "sync-review", agents: [makeAgent({ id: "agent-a" })] }),
    connectedClientCount: 1,
    multiAgentMode: false,
    sessionStatusMode: "idle",
    hotkeyToggleLabel: "Ctrl+T",
    workspaceLiveSyncStatus: workspaceLiveSyncStatus("conflict"),
  }), "Session sync-review • 1 CLI connected • 1 visible agent • sync managed conflict • Ctrl+T hotkeys")
})

function workspaceLiveSyncStatus(
  footerState: WorkspaceLiveSyncStatus["footer_state"],
): WorkspaceLiveSyncStatus {
  return {
    session_id: "session-1",
    mode: footerState === "off" ? "unrestricted" : "managed",
    footer_state: footerState,
    sync_groups: [],
    targets: [],
    conflicts: [],
    ignore: {
      rules: [],
      force_excludes: [],
    },
  }
}
