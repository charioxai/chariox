import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  CharioxMcpServerConfig,
  CharioxSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  daemonHealth,
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand rejects deprecated metaagent spawns", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
  })
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called for deprecated metaagent spawn")
  })

  const result = await executeShellCommand(parseShellCommand("agent spawn meta gpt-5.2 --meta"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /creating separate metaagents is deprecated/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand rejects deprecated metaagent spawns before slice handling", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
  })
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called for invalid metaagent slice placement")
  })

  const result = await executeShellCommand(parseShellCommand("agent spawn meta --meta --slice new"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /creating separate metaagents is deprecated/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand batch spawns agents with count option", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, {
      SpawnAgents: {
        session_id: "session-1",
        agents: [
          {
            provider: "codex",
            alias: "reviewer",
            model: "gpt-5.4",
            effort: "medium",
            execution_mode: null,
            permission_level: null,
            worktree_id: null,
            kernel_ref: null,
            slice_ref: null,
            worktree_placement: null,
          },
          {
            provider: "codex",
            alias: "reviewer-2",
            model: "gpt-5.4",
            effort: "medium",
            execution_mode: null,
            permission_level: null,
            worktree_id: null,
            kernel_ref: null,
            slice_ref: null,
            worktree_placement: null,
          },
          {
            provider: "codex",
            alias: "reviewer-3",
            model: "gpt-5.4",
            effort: "medium",
            execution_mode: null,
            permission_level: null,
            worktree_id: null,
            kernel_ref: null,
            slice_ref: null,
            worktree_placement: null,
          },
        ],
      },
    })
    return {
      AgentsSpawned: {
        agents: [
          makeAgent({ id: "agent-1", agent_ref: "agent-1", alias: "reviewer" }),
          makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer-2" }),
          makeAgent({ id: "agent-3", agent_ref: "agent-3", alias: "reviewer-3" }),
        ],
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("agents spawn reviewer --count 3"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /spawned 3 agents \(agent-1..agent-3\)/)
  assert.deepEqual(result.contextUpdates, { agentId: "agent-3" })
  assert.equal(fake.requests.length, 1)
})

test("executeShellCommand requires confirmation for large batch agent spawn", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
  })
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called before large batch spawn confirmation")
  })

  const result = await executeShellCommand(parseShellCommand("agents spawn 50 reviewer"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /spawning 50 agents requires confirmation/)
  assert.match(result.message ?? "", /--confirm-large/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand accepts confirmed large batch agent spawn", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
  })
  const fake = fakeClient((request) => {
    if ("SpawnAgents" in request) {
      const agents = (request as { SpawnAgents: { agents: Array<{ alias: string | null }> } }).SpawnAgents.agents
      assert.equal(agents.length, 50)
      assert.equal(agents[0]?.alias, "reviewer")
      assert.equal(agents[49]?.alias, "reviewer-50")
      return {
        AgentsSpawned: {
          agents: Array.from({ length: 50 }, (_, index) => makeAgent({
            id: `agent-${index + 1}`,
            agent_ref: `agent-${index + 1}`,
            alias: index === 0 ? "reviewer" : `reviewer-${index + 1}`,
          })),
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })

  const result = await executeShellCommand(parseShellCommand("agents spawn 50 reviewer --confirm-large"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /spawned 50 agents \(agent-1..agent-50\)/)
  assert.deepEqual(result.contextUpdates, { agentId: "agent-50" })
  assert.equal(fake.requests.length, 1)
})

test("executeShellCommand batch spawns and prompts agents with bounded concurrency", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attachment-1",
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
  })
  const fake = fakeClient((request) => {
    if ("SpawnAgents" in request) {
      return {
        AgentsSpawned: {
          agents: [
            makeAgent({ id: "agent-1", agent_ref: "agent-1", alias: "reviewer", provider: "opencode", model: "gpt-5.5", effort: "high" }),
            makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer-2", provider: "opencode", model: "gpt-5.5", effort: "high" }),
            makeAgent({ id: "agent-3", agent_ref: "agent-3", alias: "reviewer-3", provider: "opencode", model: "gpt-5.5", effort: "high" }),
          ],
        },
      }
    }
    if ("LaunchProviderRuns" in request) {
      const launches = (request as { LaunchProviderRuns: { launches: Array<{ agent_id: string }> } }).LaunchProviderRuns.launches
      return {
        ProviderRunsLaunchAccepted: {
          provider_runs: launches.map((launch, index) => ({
            index,
            agent_id: launch.agent_id,
            provider_run: { id: `run-${index + 1}` },
            reused: false,
          })),
          failures: [],
        },
      }
    }
    if ("SubmitPrompts" in request) {
      const prompts = (request as { SubmitPrompts: { prompts: Array<{ target_agent_id: string }> } }).SubmitPrompts.prompts
      return {
        PromptsSubmitted: {
          results: prompts.map((prompt, index) => ({
            index,
            agent_id: prompt.target_agent_id,
            outcome: {},
          })),
          failures: [],
          session: makeSession(),
          agent_activity: {},
          agent_activity_revision: 1,
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })

  const result = await executeShellCommand(
    parseShellCommand('agents spawn 3 reviewer --provider opencode --model gpt-5.5 --effort high --prompt "inspect the branch" --concurrency 2'),
    context,
    { client: fake.client },
  )

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /spawned 3 agents/)
  assert.match(result.message ?? "", /prompted 3 agents with concurrency 2/)
  assert.deepEqual(fake.requests[0], {
    SpawnAgents: {
      session_id: "session-1",
      agents: [
        {
          provider: "opencode",
          alias: "reviewer",
          model: "gpt-5.5",
          effort: "high",
          execution_mode: null,
          permission_level: null,
          worktree_id: null,
          kernel_ref: null,
          slice_ref: null,
          worktree_placement: null,
        },
        {
          provider: "opencode",
          alias: "reviewer-2",
          model: "gpt-5.5",
          effort: "high",
          execution_mode: null,
          permission_level: null,
          worktree_id: null,
          kernel_ref: null,
          slice_ref: null,
          worktree_placement: null,
        },
        {
          provider: "opencode",
          alias: "reviewer-3",
          model: "gpt-5.5",
          effort: "high",
          execution_mode: null,
          permission_level: null,
          worktree_id: null,
          kernel_ref: null,
          slice_ref: null,
          worktree_placement: null,
        },
      ],
    },
  })
  assert.deepEqual(fake.requests.map((request) => Object.keys(request)[0]), ["SpawnAgents", "LaunchProviderRuns", "SubmitPrompts"])
  const launchRequest = fake.requests[1] as { LaunchProviderRuns: { max_concurrency: number; launches: Array<{ agent_id: string; provider: string; model: string; variant: string }> } }
  assert.equal(launchRequest.LaunchProviderRuns.max_concurrency, 2)
  assert.deepEqual(launchRequest.LaunchProviderRuns.launches.map((launch) => [launch.agent_id, launch.provider, launch.model, launch.variant]), [
    ["agent-1", "opencode", "gpt-5.5", "high"],
    ["agent-2", "opencode", "gpt-5.5", "high"],
    ["agent-3", "opencode", "gpt-5.5", "high"],
  ])
  const promptRequest = fake.requests[2] as { SubmitPrompts: { max_concurrency: number; prompts: Array<{ target_agent_id: string; prompt: string; attachments: unknown[] }> } }
  assert.equal(promptRequest.SubmitPrompts.max_concurrency, 2)
  assert.deepEqual(promptRequest.SubmitPrompts.prompts, [
    { session_id: null, attachment_id: null, target_agent_id: "agent-1", prompt: "inspect the branch\n", attachments: [] },
    { session_id: null, attachment_id: null, target_agent_id: "agent-2", prompt: "inspect the branch\n", attachments: [] },
    { session_id: null, attachment_id: null, target_agent_id: "agent-3", prompt: "inspect the branch\n", attachments: [] },
  ])
})

test("executeShellCommand summarizes batch prompt failures without flooding output", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attachment-1",
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
  })
  const fake = fakeClient((request) => {
    if ("SpawnAgents" in request) {
      return {
        AgentsSpawned: {
          agents: [
            makeAgent({ id: "agent-1", agent_ref: "agent-1", alias: "reviewer" }),
            makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer-2" }),
            makeAgent({ id: "agent-3", agent_ref: "agent-3", alias: "reviewer-3" }),
          ],
        },
      }
    }
    if ("LaunchProviderRuns" in request) {
      const launches = (request as { LaunchProviderRuns: { launches: Array<{ agent_id: string }> } }).LaunchProviderRuns.launches
      return {
        ProviderRunsLaunchAccepted: {
          provider_runs: launches.map((launch, index) => ({
            index,
            agent_id: launch.agent_id,
            provider_run: { id: `run-${index + 1}` },
            reused: false,
          })),
          failures: [],
        },
      }
    }
    if ("SubmitPrompts" in request) {
      const prompts = (request as { SubmitPrompts: { prompts: Array<{ target_agent_id: string }> } }).SubmitPrompts.prompts
      return {
        PromptsSubmitted: {
          results: prompts
            .map((prompt, index) => ({ prompt, index }))
            .filter(({ prompt }) => prompt.target_agent_id !== "agent-2")
            .map(({ prompt, index }) => ({
              index,
              agent_id: prompt.target_agent_id,
              outcome: {},
            })),
          failures: [
            {
              index: 1,
              agent_id: "agent-2",
              message: "provider launch window closed",
            },
          ],
          session: makeSession(),
          agent_activity: {},
          agent_activity_revision: 1,
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })

  const result = await executeShellCommand(
    parseShellCommand('agents spawn 3 reviewer --prompt "inspect the branch" --concurrency 2'),
    context,
    { client: fake.client },
  )

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /spawned 3 agents/)
  assert.match(result.message ?? "", /prompted 2 agents with concurrency 2/)
  assert.match(result.message ?? "", /failed to prompt 1 agents \(agent-2: provider launch window closed\)/)
  assert.deepEqual(result.contextUpdates, { agentId: "agent-3" })
})

test("executeShellCommand marks agents in Meta mode in agent lists", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetSessionState: { session_id: "session-1" } })
    return {
      SessionState: {
        session: makeSession({
          agents: [
            makeAgent({ id: "agent-1", agent_ref: "agent-1" }),
            makeAgent({
              id: "agent-meta",
              agent_ref: "agent-meta",
              alias: "meta",
              role: "standard",
              meta_mode: { activated_at_ms: 1 },
            }),
          ],
        }),
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-meta \(meta\) \[Meta mode\] \[/)
  assert.equal(fake.requests.length, 1)
})

test("executeShellCommand submits prompt without waiting", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent()] } }
    }
    if ("SubmitPrompt" in request) {
      return {
        PromptSubmitted: {
          outcome: {
            Started: {
              prompt: {
                id: "prompt-1",
                source_attachment_id: "attach-1",
                target_agent_id: "agent-1",
                prompt: "hello\n",
                status: "Running",
              },
            },
          },
          session: makeSession({
            prompt_states: {
              "agent-1": {
                active_prompt: {
                  id: "prompt-1",
                  source_attachment_id: "attach-1",
                  target_agent_id: "agent-1",
                  prompt: "hello\n",
                  status: "Running",
                },
                queued_prompts: [],
              },
            },
          }),
          agent_activity: {
            "agent-1": {
              status: "working",
              prompt_status: "running",
              busy: true,
            },
          },
          agent_activity_revision: 12,
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("prompt hello"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-1 submitted/)
  assert.deepEqual((result.data as { session?: { agent_activity?: unknown } } | undefined)?.session?.agent_activity, {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
    },
  })
  assert.equal((result.data as { session?: { agent_activity_revision?: unknown } } | undefined)?.session?.agent_activity_revision, 12)
  assert.deepEqual(result.contextUpdates, { agentId: "agent-1" })
  assert.deepEqual(fake.requests.map((request) => Object.keys(request)[0]), ["ListAgents", "SubmitPrompt"])
})

test("executeShellCommand waits for prompt and renders summary blob", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
  })
  let stateCalls = 0
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent()] } }
    }
    if ("SubmitPrompt" in request) {
      return {
        PromptSubmitted: {
          outcome: {
            Started: {
              prompt: {
                id: "prompt-1",
                source_attachment_id: "attach-1",
                target_agent_id: "agent-1",
                prompt: "hello\n",
                status: "Running",
              },
            },
          },
          session: makeSession(),
          agent_activity: {},
        },
      }
    }
    if ("PumpTerminalOutput" in request) {
      return { TerminalOutputPumped: { records: [] } }
    }
    if ("GetSessionState" in request) {
      stateCalls += 1
      return {
        SessionState: {
          session: makeSession({
            prompt_states: {
              "agent-1": {
                active_prompt: stateCalls === 1
                  ? {
                      id: "prompt-1",
                      source_attachment_id: "attach-1",
                      target_agent_id: "agent-1",
                      prompt: "hello\n",
                      status: "Running",
                    }
                  : null,
                queued_prompts: [],
              },
            },
          }),
        },
      }
    }
    if ("GetSessionHistoryOutline" in request) {
      return {
        SessionHistoryOutline: {
          agents: [{
            agent_id: "agent-1",
            turns: [{
              turn_id: "turn-1",
              started_at_ms: 1,
              user_prompt: {
                entry_index: 1,
                fragment_start: 0,
                fragment_end: 5,
                total_chars: 5,
                entry: { agent_id: "agent-1", kind: "user_prompt", text: "hello\n" },
              },
              entries: [],
              blobs: [],
              summary: {
              entry_index: 2,
              fragment_start: 0,
              fragment_end: 7,
              total_chars: 7,
              entry: { agent_id: "agent-1", kind: "provider_output", text: "done ok" },
              },
            }],
            next_cursor: null,
          }],
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("prompt hello --wait --show-summary"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-1 completed/)
  assert.match(result.message ?? "", /prompt-1 summary\n {24}done ok/)
})

test("executeShellCommand waits for queued prompts until they run and settle", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
  })
  let stateCalls = 0
  const queuedPrompt = {
    id: "prompt-queued",
    source_attachment_id: "attach-1",
    target_agent_id: "agent-1",
    prompt: "hello\n",
    status: "Queued",
  }
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent()] } }
    }
    if ("SubmitPrompt" in request) {
      return {
        PromptSubmitted: {
          outcome: { Queued: { prompt: queuedPrompt } },
          session: makeSession({
            prompt_states: {
              "agent-1": {
                active_prompt: null,
                queued_prompts: [queuedPrompt],
              },
            },
          }),
          agent_activity: {
            "agent-1": {
              status: "idle",
              prompt_status: "queued",
              busy: false,
              queued_prompt_count: 1,
            },
          },
        },
      }
    }
    if ("PumpTerminalOutput" in request) {
      return { TerminalOutputPumped: { records: [] } }
    }
    if ("GetSessionState" in request) {
      stateCalls += 1
      return {
        SessionState: {
          session: makeSession({
            prompt_states: {
              "agent-1": stateCalls === 1
                ? {
                    active_prompt: null,
                    queued_prompts: [queuedPrompt],
                  }
                : {
                    active_prompt: stateCalls === 2
                      ? { ...queuedPrompt, status: "Running" }
                      : null,
                    queued_prompts: [],
                  },
            },
          }),
        },
      }
    }
    if ("GetSessionHistoryOutline" in request) {
      return {
        SessionHistoryOutline: {
          agents: [{
            agent_id: "agent-1",
            turns: [{
              turn_id: "turn-1",
              started_at_ms: 1,
              user_prompt: {
                entry_index: 1,
                fragment_start: 0,
                fragment_end: 5,
                total_chars: 5,
                entry: { agent_id: "agent-1", kind: "user_prompt", text: "hello\n" },
              },
              entries: [],
              blobs: [],
              summary: {
                entry_index: 2,
                fragment_start: 0,
                fragment_end: 4,
                total_chars: 4,
                entry: { agent_id: "agent-1", kind: "provider_output", text: "done" },
              },
            }],
            next_cursor: null,
          }],
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("prompt hello --wait"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-queued completed/)
  assert.equal(stateCalls, 3)
})

test("executeShellCommand wait trusts projected idle activity over stale prompt state", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
  })
  let stateCalls = 0
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent()] } }
    }
    if ("SubmitPrompt" in request) {
      return {
        PromptSubmitted: {
          outcome: {
            Started: {
              prompt: {
                id: "prompt-1",
                source_attachment_id: "attach-1",
                target_agent_id: "agent-1",
                prompt: "hello\n",
                status: "Running",
              },
            },
          },
          session: makeSession(),
          agent_activity: {
            "agent-1": {
              status: "working",
              prompt_status: "running",
              busy: true,
            },
          },
        },
      }
    }
    if ("PumpTerminalOutput" in request) {
      return { TerminalOutputPumped: { records: [] } }
    }
    if ("GetSessionState" in request) {
      stateCalls += 1
      return {
        SessionState: {
          session: makeSession({
            prompt_states: {
              "agent-1": {
                active_prompt: {
                  id: "prompt-1",
                  source_attachment_id: "attach-1",
                  target_agent_id: "agent-1",
                  prompt: "hello\n",
                  status: "Running",
                },
                queued_prompts: [],
              },
            },
          }),
          agent_activity: {
            "agent-1": {
              status: "idle",
              prompt_status: "none",
              busy: false,
            },
          },
        },
      }
    }
    if ("GetSessionHistoryOutline" in request) {
      return {
        SessionHistoryOutline: {
          agents: [{
            agent_id: "agent-1",
            turns: [{
              turn_id: "turn-1",
              started_at_ms: 1,
              user_prompt: {
                entry_index: 1,
                fragment_start: 0,
                fragment_end: 5,
                total_chars: 5,
                entry: { agent_id: "agent-1", kind: "user_prompt", text: "hello\n" },
              },
              entries: [],
              blobs: [],
              summary: {
                entry_index: 2,
                fragment_start: 0,
                fragment_end: 4,
                total_chars: 4,
                entry: { agent_id: "agent-1", kind: "provider_output", text: "done" },
              },
            }],
            next_cursor: null,
          }],
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("prompt hello --wait"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-1 completed/)
  assert.equal(stateCalls, 1)
})

test("executeShellCommand renders provider tools through shared tool display for show-reply", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
  })
  let stateCalls = 0
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent()] } }
    }
    if ("SubmitPrompt" in request) {
      return {
        PromptSubmitted: {
          outcome: {
            Started: {
              prompt: {
                id: "prompt-1",
                source_attachment_id: "attach-1",
                target_agent_id: "agent-1",
                prompt: "hello\n",
                status: "Running",
              },
            },
          },
          session: makeSession(),
          agent_activity: {},
        },
      }
    }
    if ("PumpTerminalOutput" in request) {
      return { TerminalOutputPumped: { records: [] } }
    }
    if ("GetSessionState" in request) {
      stateCalls += 1
      return {
        SessionState: {
          session: makeSession({
            prompt_states: {
              "agent-1": {
                active_prompt: stateCalls === 1
                  ? {
                      id: "prompt-1",
                      source_attachment_id: "attach-1",
                      target_agent_id: "agent-1",
                      prompt: "hello\n",
                      status: "Running",
                    }
                  : null,
                queued_prompts: [],
              },
            },
          }),
        },
      }
    }
    if ("GetSessionHistoryOutline" in request) {
      return {
        SessionHistoryOutline: {
          agents: [{
            agent_id: "agent-1",
            turns: [{
              turn_id: "turn-1",
              started_at_ms: 1,
              user_prompt: {
                entry_index: 1,
                fragment_start: 0,
                fragment_end: 5,
                total_chars: 5,
                entry: { agent_id: "agent-1", kind: "user_prompt", text: "hello\n" },
              },
              entries: [],
              blobs: [{
                blob_id: "history:2:2",
                kind: "provider_tool",
                title: "tool",
                summary: "read seed.txt",
                sequence_start: 2,
                sequence_end: 2,
                entry_count: 1,
                total_chars: 100,
                timestamp_ms: 2,
              }],
              summary: {
                entry_index: 3,
                fragment_start: 0,
                fragment_end: 7,
                total_chars: 7,
                entry: { agent_id: "agent-1", kind: "provider_output", text: "done ok" },
              },
            }],
            next_cursor: null,
          }],
        },
      }
    }
    if ("GetSessionHistoryBlobContent" in request) {
      return {
        SessionHistoryBlobContent: {
          blob_id: "history:2:2",
          entries: [{
              entry_index: 2,
              fragment_start: 0,
              fragment_end: 100,
              total_chars: 100,
              entry: {
                agent_id: "agent-1",
                kind: "provider_tool",
                text: JSON.stringify({
                  id: "tool-read",
                  tool: "chariox_read_artifact",
                  status: "completed",
                  input: { path: "seed.txt", domain: "text" },
                  output: JSON.stringify({ content_text: "TOOL_DISPLAY_FIXTURE_SEED\n", path: "seed.txt", domain: "text" }),
                }),
              },
            }],
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("prompt hello --wait --show-reply"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /\*\*read\*\* · COMPLETED/)
  assert.match(result.message ?? "", /TOOL_DISPLAY_FIXTURE_SEED/)
  assert.doesNotMatch(result.message ?? "", /\[provider_tool\]/)
})
