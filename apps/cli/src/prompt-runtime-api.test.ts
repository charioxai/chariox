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

test("interaction responses must match both requested session and interaction", async () => {
  for (const [sessionId, interactionId] of [["other", "approval"], ["session-1", "other"], ["session-1", undefined]]) {
    await assert.rejects(respondToInteraction(fakeClient({ InteractionResponded: {
      session: runtimeSession(sessionId!), interaction_id: interactionId,
    } }), "session-1", "approval", "allow", null), /identity mismatch/)
  }
  const result = await respondToInteraction(fakeClient({ InteractionResponded: {
    session: runtimeSession("session-1"), interaction_id: "approval",
  } }), "session-1", "approval", "allow", null)
  assert.equal(result.id, "session-1")
})

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

// MP-08 / MP-10 / MP-11: v2 selection identity stays on the existing prompt lifecycle.
test("Skills selections submit once with typed source identity and authoritative activity", async () => {
    const requests: Record<string, unknown>[] = [];
    const selection = { source: "chariox" as const, skill_id: "skill", name: "review", catalog_revision: 7 };
    const client = { send: async (request: Record<string, unknown>) => { if ("RelayStatus" in request) return {RelayStatus:{status:{capabilities:["skills_v2_runtime"]}}}; requests.push(request); return "SkillsV2" in request ? { SkillsV2: { result: { kind: "submitted", outcome: { Started: { prompt: { id: "prompt", source_attachment_id: "attachment", target_agent_id: "agent-1", prompt: "$review", status: "Running" } } } } } } : { SessionState: { session: runtimeSession("session-1"), agent_activity: { "agent-1": { status: "working", busy: true } }, agent_activity_revision: 3 } }; } } as unknown as LocalIpcClient;
    const result = await submitPromptWithRecovery(client, "session-1", "attachment", "agent-1", "$review", [], () => runtimeSession("session-1"), {} as CliOptions, null, [selection]);
    assert.equal(requests.length, 2);
    assert.deepEqual((requests[0] as {
        SkillsV2: {
            selections: unknown[];
        };
    }).SkillsV2.selections, [selection]);
    assert.equal(result.payload.session.agent_activity_revision, 3);
});
test("Skills refusal does not fall through to the retired prompt/grant path", async () => {
    const requests: unknown[] = [];
    const client = { send: async (request: any) => { if ("RelayStatus" in request) return {RelayStatus:{status:{capabilities:["skills_v2_runtime"]}}}; requests.push(request); throw Error("Skills revision changed"); } } as unknown as LocalIpcClient;
    await assert.rejects(() => submitPromptWithRecovery(client, "session-1", "attachment", "agent-1", "$review", [], () => runtimeSession("session-1"), {} as CliOptions, null, []), /revision changed/);
    assert.equal(requests.length, 1);
    assert.ok("SkillsV2" in (requests[0] as object));
});

// MP-08 / MP-10 / MP-11: compatible kernels need no Skills feature for dollars.
test("literal dollars use SubmitPrompt on a pre-533 kernel", async () => {
  for (const prompt of ["Explain $HOME", "Pay $20", String.raw`Escape \$review`, "Use `$review`", "```sh\n$review\n```", "Only $", "Explain the string \"hello $review world\"", "Explain 'hello $review world'", "    $review", "Paragraph\n    $review", "[read $review](https://example.com)", "[read](https://example.com/ $review )", "~~~\n$review\n~~~ not a closing fence\n$review", "``a ` $review``"]) {
    const requests: unknown[]=[]
    const client={send:async (request:Record<string,unknown>)=>{requests.push(request); if("SkillsV2" in request) throw Error("requires protocol 533");return {PromptSubmitted:{session:runtimeSession("session-1"),agent_activity:{},agent_activity_revision:1}}}} as unknown as LocalIpcClient
    await submitPromptWithRecovery(client,"session-1","attachment","agent-1",prompt,[],()=>runtimeSession("session-1"),{} as CliOptions)
    assert.ok("SubmitPrompt" in (requests[0] as object),prompt)
  }
})

// MP-08 / MP-10 / MP-11: a protocol foundation is not runtime admission.
test("plain dollar skill text retains SubmitPrompt without advertised runtime", async () => {
    const requests: Record<string, unknown>[] = [];
    const client = { send: async (request: Record<string, unknown>) => {
        requests.push(request);
        if ("RelayStatus" in request) return { RelayStatus: { status: { capabilities: [] } } };
        if ("SkillsV2" in request) throw Error("foundation endpoint must not be called");
        return { PromptSubmitted: { session: runtimeSession("session-1"), agent_activity: {}, agent_activity_revision: 1 } };
    } } as unknown as LocalIpcClient;
    await submitPromptWithRecovery(client, "session-1", "attachment", "agent-1", "Use $review", [], () => runtimeSession("session-1"), {} as CliOptions);
    assert.equal(requests.filter(r => "SkillsV2" in r).length, 0);
    assert.deepEqual(requests.at(-1), { SubmitPrompt: { session_id: "session-1", attachment_id: "attachment", target_agent_id: "agent-1", prompt: "Use $review", attachments: [] } });
});

// MP-08 / MP-10 / MP-11: never downgrade typed source identities.
test("typed TUI chip refuses unadvertised runtime without a turn", async () => {
  const calls:unknown[]=[];
  const client={send:async(request:unknown)=>{calls.push(request);return {RelayStatus:{status:{capabilities:[]}}}}} as unknown as LocalIpcClient;
  await assert.rejects(()=>submitPromptWithRecovery(client,"session-1","attachment","agent-1","Use $review",[],()=>runtimeSession("session-1"),{} as CliOptions,null,[{source:"chariox",skill_id:"s",name:"review",catalog_revision:1}]),/runtime.*unavailable/);
  assert.deepEqual(calls,[{RelayStatus:null}]);
});
