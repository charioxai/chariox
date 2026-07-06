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
    () => runtimeSession("session-1"),
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

test("submitPromptWithRecovery recovers with the stored provider for the prompt target", async () => {
  const requests: Record<string, unknown>[] = []
  let submitAttempts = 0
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("SubmitPrompt" in request) {
        submitAttempts += 1
        if (submitAttempts === 1) {
          throw new Error("session has no active provider run")
        }
        return {
          PromptSubmitted: {
            outcome: {
              Started: {
                prompt: {
                  id: "prompt-1",
                  source_attachment_id: "attachment-1",
                  target_agent_id: "agent-b",
                  prompt: "hello",
                  status: "Running",
                },
              },
            },
            session: runtimeSession("session-1"),
          },
        }
      }
      if ("LaunchProviderRun" in request) {
        return {
          ProviderRunLaunched: {
            provider_run: {
              id: "run-1",
              session_id: "session-1",
              provider: "claude",
              model: "claude/sonnet-4.6",
              variant: "high",
              status: "Running",
            },
          },
        }
      }
      throw new Error(`unexpected request ${JSON.stringify(request)}`)
    },
  } as unknown as LocalIpcClient

  await submitPromptWithRecovery(
    client,
    "session-1",
    "attachment-1",
    "agent-b",
    "hello",
    [],
    () => runtimeSession("session-1", {
      focused_agent_id: "agent-a",
      agents: [
        agent("agent-a", { provider: "codex", model: "codex/gpt-5", effort: "medium" }),
        agent("agent-b", { provider: "claude", model: "claude/sonnet-4.6", effort: "high" }),
      ],
    }),
    {
      provider: "opencode",
      accountProfile: "default",
      model: "kimi/k2.6",
      effort: "low",
    } as CliOptions,
  )

  const launchRequest = requests.find((request) => "LaunchProviderRun" in request)
  assert.deepEqual(launchRequest, {
    LaunchProviderRun: {
      session_id: "session-1",
      agent_id: "agent-b",
      adapter_key: "claude",
      provider: "claude",
      account_profile: "default",
      model: "sonnet-4.6",
      variant: "high",
      structured_endpoint: null,
      provider_session_id: null,
      native_tui: false,
    },
  })
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

function runtimeSession(id: string, overrides: Partial<RuntimeSession> = {}): RuntimeSession {
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
    agents: [agent("agent-1")],
    config_state: {
      version: 1,
      values: {},
    },
    ...overrides,
  }
}

function agent(id: string, overrides: Partial<RuntimeSession["agents"][number]> = {}): RuntimeSession["agents"][number] {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: id,
    provider: "codex",
    model: "gpt-5.2",
    effort: "medium",
    worktree_id: "/workspace/tree",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}
