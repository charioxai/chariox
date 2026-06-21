import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, TranscriptEntry } from "./cli-types.js"
import {
  syncQueuedPromptEntriesForAgent,
} from "./queued-prompt-transcript.js"

test("syncQueuedPromptEntriesForAgent appends queued prompts and removes settled queued entries", () => {
  const existing: TranscriptEntry[] = [
    { id: 1, role: "assistant", text: "ready" },
    {
      id: 2,
      role: "user",
      text: "old queued",
      queuedPrompt: { agentId: "agent-1", promptId: "old-prompt", status: "queued" },
    },
  ]

  const synced = syncQueuedPromptEntriesForAgent(existing, sessionWithQueuedPrompt(), "agent-1")

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries.map((entry) => entry.queuedPrompt?.promptId).filter(Boolean), [
    "prompt-1",
  ])
  assert.equal(synced.entries.at(-1)?.text, "new queued")
})

function sessionWithQueuedPrompt(): RuntimeSession {
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
  }
}
