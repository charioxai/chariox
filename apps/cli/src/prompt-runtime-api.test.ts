import assert from "node:assert/strict"
import test from "node:test"

import type {
  CliOptions,
  RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  cancelQueuedPrompt,
  steerQueuedPrompt,
  submitPromptWithRecovery,
} from "./prompt-runtime-api.js"

test("submitPromptWithRecovery merges projected agent activity into returned session", async () => {
  const client = fakeClient({
    PromptSubmitted: {
      outcome: {
        Started: {
          prompt: {
            id: "prompt-1",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: "Running",
          },
        },
      },
      session: runtimeSession("session-1"),
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
        },
      },
      agent_activity_revision: 42,
    },
  })

  const result = await submitPromptWithRecovery(
    client,
    "session-1",
    "attachment-1",
    "agent-1",
    "hello",
    [],
    {} as CliOptions,
  )

  assert.deepEqual(result.payload.session.agent_activity, {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
    },
  })
  assert.equal(result.payload.session.agent_activity_revision, 42)
  assert.equal(result.targetAgentId, "agent-1")
})

test("steerQueuedPrompt merges projected agent activity into returned session", async () => {
  const client = fakeClient({
    QueuedPromptSteered: {
      prompt: {
        id: "queued-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "steer",
        status: "Steered",
      },
      session: runtimeSession("session-1"),
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
        },
      },
      agent_activity_revision: 43,
    },
  })

  const payload = await steerQueuedPrompt(client, "session-1", "attachment-1", "agent-1", "queued-1")

  assert.deepEqual(payload.session.agent_activity, {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
    },
  })
  assert.equal(payload.session.agent_activity_revision, 43)
})

test("cancelQueuedPrompt merges projected agent activity into returned session", async () => {
  const client = fakeClient({
    QueuedPromptCancelled: {
      prompt: {
        id: "queued-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "cancel",
        status: "Cancelled",
      },
      session: runtimeSession("session-1"),
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
        },
      },
      agent_activity_revision: 44,
    },
  })

  const payload = await cancelQueuedPrompt(client, "session-1", "attachment-1", "agent-1", "queued-1")

  assert.deepEqual(payload.session.agent_activity, {
    "agent-1": {
      status: "idle",
      prompt_status: "none",
      busy: false,
    },
  })
  assert.equal(payload.session.agent_activity_revision, 44)
})

function fakeClient(response: Record<string, unknown>): LocalIpcClient {
  return {
    send: async () => response,
  } as unknown as LocalIpcClient
}

function runtimeSession(id: string): RuntimeSession {
  return {
    id,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 1,
    agents: [{
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: id,
      alias: "agent-1",
      provider: "codex",
      model: "gpt-5.2",
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
