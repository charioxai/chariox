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
  respondToInteraction,
  submitPromptWithRecovery,
} from "./prompt-runtime-api.js"

test("an interaction answer is an acknowledgement for the requested interaction and choice; the terminal then reads the Room", async () => {
  for (const [interactionId, choiceId] of [["other", "allow"], ["approval", "other"], [undefined, "allow"]]) {
    await assert.rejects(respondToInteraction(fakeClient({ InteractionResponded: {
      interaction_id: interactionId, choice_id: choiceId, outcome: "answered",
    } }), "session-1", "approval", "allow", null), /identity mismatch/)
  }
  const sent: Array<Record<string, unknown>> = []
  const client = { send: async (request: Record<string, unknown>) => {
    sent.push(request)
    return "RespondToInteraction" in request
      ? { InteractionResponded: { interaction_id: "approval", choice_id: "allow", outcome: "answered" } }
      : { SessionState: { session: runtimeSession("session-1"), processes: [] } }
  } } as unknown as LocalIpcClient
  const result = await respondToInteraction(client, "session-1", "approval", "allow", null)
  assert.equal(result.id, "session-1")
  assert.deepEqual(sent.map(request => Object.keys(request)[0]), ["RespondToInteraction", "GetSessionState"])
})

test("submitPromptWithRecovery reads Room activity after accepting the receipt", async () => {
  const activity = { "agent-1": { status: "working", prompt_status: "running", busy: true } }
  const requests: string[] = []
  const client = { send: async (request: Record<string, unknown>) => {
    requests.push(Object.keys(request)[0]!)
    if ("SubmitPrompt" in request) return {
      PromptSubmitted: {
        session_id: "session-1",
        outcome: {
          Started: {
            prompt: { id: "prompt-1", target_agent_id: "agent-1", status: "Running", created_at_ms: 1, updated_at_ms: 1 },
          },
        },
      },
    }
    return { SessionState: { session: runtimeSession("session-1"), agent_activity: activity, agent_activity_revision: 42, processes: [] } }
  } } as unknown as LocalIpcClient
  const result = await submitPromptWithRecovery(
    client, "session-1", "attachment-1", "agent-1", "hello", [],
    () => runtimeSession("session-1"), {} as CliOptions,
  )
  assert.deepEqual(result.payload.session.agent_activity, activity)
  assert.equal(result.payload.session.agent_activity_revision, 42)
  assert.equal(result.targetAgentId, "agent-1")
  assert.deepEqual(requests, ["SubmitPrompt", "GetSessionState"])
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
                  target_agent_id: "agent-b",
                  created_at_ms: 1,
                  updated_at_ms: 1,
                  status: "Running",
                },
              },
            },
            session_id: "session-1",
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

  assert.equal(submitAttempts, 2, "a state read failure must not repeat the accepted submission")
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

for (const [tag, request, status] of [
  ["QueuedPromptSteered", steerQueuedPrompt, "Running"],
  ["QueuedPromptCancelled", cancelQueuedPrompt, "Cancelled"],
] as const) {
  test(tag + " consumes the matching queue receipt", async () => {
    const prompt = { id: "queued-1", target_agent_id: "agent-1", status, created_at_ms: 1, updated_at_ms: 2 }
    const receipt = { session_id: "session-1", prompt }
    const result = await request(fakeClient({ [tag]: receipt }), "session-1", "attachment-1", "agent-1", "queued-1")
    assert.deepEqual(result, receipt)
    await assert.rejects(
      request(fakeClient({ [tag]: { ...receipt, session_id: "other" } }), "session-1", "attachment-1", "agent-1", "queued-1"),
      /did not confirm this queued prompt/,
    )
  })
}

function fakeClient(response: Record<string, unknown>): LocalIpcClient {
  return {
    send: async () => response,
  } as unknown as LocalIpcClient
}

function runtimeSession(id: string, overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id,
    project_id: "project-default",
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
