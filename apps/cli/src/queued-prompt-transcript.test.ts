import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, TranscriptEntry } from "./cli-types.js"
import {
  syncQueuedPromptEntriesByAgent,
  syncQueuedPromptEntriesForAgent,
} from "./queued-prompt-transcript.js"

test("syncQueuedPromptEntriesForAgent keeps live queued prompts out of transcript scrollback", () => {
  const synced = syncQueuedPromptEntriesForAgent([
    { id: 1, role: "assistant", text: "ready" },
  ], sessionWithQueuedPrompt(), "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, [
    { id: 1, role: "assistant", text: "ready" },
  ])
})

test("syncQueuedPromptEntriesForAgent removes legacy queued transcript rows when projection is authoritative", () => {
  const existing: TranscriptEntry[] = [
    { id: 7, role: "assistant", text: "ready" },
    {
      id: 8,
      role: "user",
      text: "legacy queued",
      queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
    },
    { id: 9, role: "assistant", text: "still here" },
  ]

  const synced = syncQueuedPromptEntriesForAgent(existing, sessionWithQueuedPrompt(), "agent-1")

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries, [
    { id: 1, role: "assistant", text: "ready" },
    { id: 2, role: "assistant", text: "still here" },
  ])
})

test("syncQueuedPromptEntriesForAgent preserves legacy queued rows when projection is unavailable", () => {
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "preserved queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-preserved"),
  }]
  const session = sessionWithoutPromptStates({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
      },
    },
  })

  const synced = syncQueuedPromptEntriesForAgent(existing, session, "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, existing)
})

test("syncQueuedPromptEntriesByAgent prunes stale queued prompt panes from projected agents", () => {
  const synced = syncQueuedPromptEntriesByAgent({
    "agent-1": [{
      id: 1,
      role: "user",
      text: "legacy queued",
      queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
    }],
  }, sessionWithQueuedPrompt({}, {
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  }))

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entriesByAgent["agent-1"], [])
  assert.deepEqual(synced.previews["agent-1"], "")
})

test("syncQueuedPromptEntriesByAgent preserves legacy queued panes without authoritative projection", () => {
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "preserved queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-preserved"),
  }]
  const session = sessionWithoutPromptStates({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
      },
    },
  })

  const synced = syncQueuedPromptEntriesByAgent({ "agent-1": existing }, session)

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entriesByAgent["agent-1"], existing)
})

function sessionWithoutPromptStates(sessionOverrides: Partial<RuntimeSession> = {}): RuntimeSession {
  const session = sessionWithQueuedPrompt({}, sessionOverrides)
  delete session.prompt_states
  return session
}

function queuedPrompt(agentId: string, promptId: string): NonNullable<TranscriptEntry["queuedPrompt"]> {
  return {
    agentId,
    promptId,
    status: "queued",
    attachmentCount: 0,
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  }
}

function sessionWithQueuedPrompt(
  overrides: Partial<NonNullable<RuntimeSession["prompt_states"]>[string]> = {},
  sessionOverrides: Partial<RuntimeSession> = {},
): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "new queued",
          status: "Queued",
        }],
        ...overrides,
      },
    },
    focused_agent_id: "agent-1",
    max_agents: 1,
    agents: [{
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: "session-1",
      alias: null,
      provider: "codex",
      model: "gpt-5",
      worktree_id: "/workspace/tree",
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: {
      version: 1,
      values: {},
    },
    ...sessionOverrides,
  }
}
