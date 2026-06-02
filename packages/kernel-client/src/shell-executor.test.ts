import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  DaemonHealthProjection,
  ProviderProcessInfo,
  WorkflowPublicationTrustedSender,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand handles shell-local context mutations", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("set model gpt-5.3"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, { model: "gpt-5.3" })
  const next = applyShellCommandResult(context, result)
  assert.equal(next.model, "gpt-5.3")
})

test("executeShellCommand help advertises workspace live sync config values", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("help"), context, { client: fakeClient(() => ({})).client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /kernel health\|status\|debug-bundle \[label\]\|delete/)
  assert.match(result.message ?? "", /config show\|path\|keys\|schema\|set\|unset\|workspace-live-sync off\|managed\|tracked/)
  assert.match(result.message ?? "", /workspace sync status\|targets\|conflicts\|ignore\|off\|managed\|tracked\|link/)
  assert.match(result.message ?? "", /slice list\|create\|status\|doctor\|logs\|start\|stop\|delete\|auth import\|auth remove\|auth login\|auth alias\|screen/)
})

test("executeShellCommand exports session-scoped kernel debug bundle", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, {
      ExportDebugBundle: {
        session_id: "session-1",
        bundle_label: "glitch",
        limit: null,
      },
    })
    return {
      DebugBundleExported: {
        bundle_dir: "/kernel/logs/debug-bundles/session-1-glitch",
        manifest_path: "/kernel/logs/debug-bundles/session-1-glitch/manifest.json",
        logs_path: "/kernel/logs/debug-bundles/session-1-glitch/logs.ndjson",
        log_root: "/kernel/logs",
        record_count: 12,
        limit: 1000,
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("kernel debug-bundle glitch"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /kernel debug bundle exported on kernel machine: \/kernel\/logs\/debug-bundles\/session-1-glitch \(12\/1000 records\)/)
  assert.equal(fake.requests.length, 1)
})

test("executeShellCommand requires an active session for kernel debug bundle", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called without a session")
  })

  const result = await executeShellCommand(parseShellCommand("kernel debug-bundle"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /requires an active session/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand renders kernel health diagnostics", async () => {
  const baseHealth = daemonHealth()
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          provider_runs: {
            ...baseHealth.provider_runs,
            projected_runs: 2,
            active_runs: 2,
            arroba_active_runs: 2,
            duplicate_arroba_agent_bindings: [{
              session_id: "session-1",
              agent_id: "agent-1",
              provider_run_ids: ["run-1", "run-2"],
            }],
          },
          remote_execution: {
            ...baseHealth.remote_execution,
            remote_agents: 1,
            active_remote_agents: 1,
            missing_active_worker_runs: 1,
            issues: [{
              kind: "missing_active_worker_provider_run",
              session_id: "session-1",
              agent_id: "agent-remote",
              agent_ref: "agent-remote",
              worker_kernel_id: "worker-kernel",
              worker_machine_id: "worker-machine",
              execution_lease_id: "lease-1",
              leased_agent_id: "leased-agent-1",
              state: "working",
              is_processing: true,
              details: "active remote agent has no worker run",
            }],
          },
          workspace_live_sync: {
            ...baseHealth.workspace_live_sync,
            managed_mode: {
              write_fence_supported: false,
              write_fence_backend: null,
              unavailable_reason: "managed mode needs selective write fencing",
            },
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /^kernel health/)
  assert.match(result.message ?? "", /command lanes: session=0\/0 agent=0\/0 workflow=0\/0 provider=0\/0 saturated=0/)
  assert.match(result.message ?? "", /provider runs: projected=2 active=2 arroba=2 native_tui=0/)
  assert.match(result.message ?? "", /remote execution: remote_agents=1 active=1 missing_worker_runs=1 malformed=0/)
  assert.match(result.message ?? "", /duplicate Arroba provider run bindings:/)
  assert.match(result.message ?? "", /session=session-1 agent=agent-1 runs=run-1,run-2/)
  assert.match(result.message ?? "", /remote execution issues: missing_worker_runs=1 malformed=0/)
  assert.match(result.message ?? "", /agent=agent-remote \(agent-remote\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-1 state=working processing=yes kind=missing_active_worker_provider_run: active remote agent has no worker run/)
  assert.match(result.message ?? "", /next: run \/agent inspect agent-remote; run \/machine kernels worker-machine; reconnect or relaunch the remote\/slice worker before sending more prompts/)
  assert.match(result.message ?? "", /workspace live sync managed mode unavailable: managed mode needs selective write fencing/)
  assert.match(result.message ?? "", /next: select tracked mode on this worker or run the managed provider on a supported host/)
})

test("executeShellCommand renders attached session runtime context before kernel health", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("GetDaemonHealth" in request) {
      return { DaemonHealth: { projection: daemonHealth() } }
    }
    assert.deepEqual(request, { GetSessionState: { session_id: "session-1" } })
    return {
      SessionState: {
        session: makeSession({
          id: "session-1",
          host_daemon_id: "home-kernel-1",
          host_machine_id: "home-machine-1",
          owner_user_id: "user-1",
        }),
        agent_activity: {},
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /^session runtime:\n  session: session-1\n  home kernel: home-kernel-1@home-machine-1\n  owner: user-1\n  agent: agent-1\nkernel health/)
  assert.deepEqual(fake.requests, [
    { GetDaemonHealth: null },
    { GetSessionState: { session_id: "session-1" } },
  ])
})

test("executeShellCommand keeps kernel health available when session runtime lookup fails", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-missing",
  })
  const fake = fakeClient((request) => {
    if ("GetDaemonHealth" in request) {
      return { DaemonHealth: { projection: daemonHealth() } }
    }
    throw new Error("session not found")
  })

  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /^session runtime:\n  session: session-missing\n  home kernel: unknown\n  lookup: session not found\nkernel health/)
})

test("executeShellCommand renders shell-local context and pwd", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo/worktree",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
    workflowId: "workflow-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
    variables: { wf: "workflow-1" },
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            host_daemon_id: "home-kernel-1",
            host_machine_id: "home-machine-1",
            owner_user_id: "user-1",
            workspace_live_sync_mode: "managed",
            active_provider_run_id: "session-run-1",
            agents: [makeAgent({
              id: "agent-1",
              agent_ref: "agent-1",
              remote_execution: {
                worker_kernel_id: "slice-kernel",
                worker_machine_id: "slice-machine",
                execution_lease_id: "lease-1",
                leased_agent_id: "leased-agent-1",
                active_worker_provider_run_id: "worker-run-1",
              },
              extension_grants: [
                { kind: "script", name: "deploy" },
                { kind: "skill", name: "review" },
              ],
              remote_extension_manifest_sync: {
                state: "stale",
                manifest_hash: "abcdef1234567890",
                last_error: "worker behind",
              },
            })],
            prompt_states: {
              "agent-1": {
                active_prompt: {
                  id: "prompt-1",
                  source_attachment_id: "attach-1",
                  target_agent_id: "agent-1",
                  prompt: "hi",
                  status: "Running",
                },
                queued_prompts: [],
              },
            },
          }),
          agent_activity: {},
        },
      }
    }
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-1",
            name: "devbox",
            owner_kernel_id: "home-kernel-1",
            owner_machine_id: "home-machine-1",
            backend: "local_docker",
            os: "linux",
            status: "running",
            worker_kernel_ref: "slice-kernel",
            worker_kernel_id: "slice-kernel",
            worker_machine_id: "slice-machine",
            agent_ids: ["agent-1"],
            created_at_ms: 0,
            updated_at_ms: 0,
          }],
        },
      }
    }
    if ("GetProviderRun" in request) {
      return { ProviderRun: { provider_run: { id: "session-run-1", agent_instance_id: "agent-1" } } }
    }
    return {}
  })
  const contextResult = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })
  const pwdResult = await executeShellCommand(parseShellCommand("pwd"), context, { client: fake.client })

  assert.equal(contextResult.ok, true)
  assert.match(contextResult.message ?? "", /workspace: \/repo/)
  assert.match(contextResult.message ?? "", /worktree: \/repo\/worktree/)
  assert.match(contextResult.message ?? "", /session: session-1/)
  assert.match(contextResult.message ?? "", /home kernel: home-kernel-1@home-machine-1/)
  assert.match(contextResult.message ?? "", /session owner: user-1/)
  assert.match(contextResult.message ?? "", /workspace live sync: managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(contextResult.message ?? "", /agent: agent-1 \(busy\)/)
  assert.match(contextResult.message ?? "", /agent placement: slice devbox \(worker=slice-machine, kernel=slice-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=worker-run-1\)/)
  assert.match(contextResult.message ?? "", /provider run: session=session-run-1, worker=worker-run-1/)
  assert.match(contextResult.message ?? "", /extensions: 2 grants \(active tools home-proxy; skills snapshot; script=1, skill=1\)/)
  assert.match(contextResult.message ?? "", /remote extension sync: stale, hash=abcdef123456, error=worker behind, next=run \/extension sync-status agent-1; run \/machine kernels slice-machine; use \/extension sync-retry agent-1/)
  assert.match(contextResult.message ?? "", /workflow: workflow-1/)
  assert.match(contextResult.message ?? "", /provider: codex/)
  assert.match(contextResult.message ?? "", /\$wf = workflow-1/)
  assert.equal(pwdResult.message, "/repo/worktree")
  assert.equal(fake.requests.length, 3)
})

test("executeShellCommand context keeps home machine visible without daemon id", async () => {
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
          host_machine_id: "home-machine-1",
        }),
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /home kernel: home-machine-1/)
  assert.match(result.message ?? "", /session owner: -/)
})

test("executeShellCommand does not infer provider run ownership from focused agent", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            active_provider_run_id: "session-run-2",
            focused_agent_id: "agent-1",
            agents: [
              makeAgent({ id: "agent-1", agent_ref: "agent-1" }),
              makeAgent({ id: "agent-2", agent_ref: "agent-2" }),
            ],
          }),
        },
      }
    }
    if ("GetProviderRun" in request) {
      return { ProviderRun: { provider_run: { id: "session-run-2", agent_instance_id: "agent-2" } } }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent: agent-1/)
  assert.match(result.message ?? "", /provider run: session=session-run-2 owned_by=agent-2/)
})

test("executeShellCommand lists sessions with home kernel ownership", async () => {
  const sessions = [
    makeSession({
      id: "session-1",
      alias: "main",
      host_daemon_id: "home-kernel-1",
      host_machine_id: "home-machine-1",
      attachment_ids: ["attachment-1"],
      worktree_id: "/repo/main",
    }),
    makeSession({
      id: "session-2",
      alias: null,
      host_machine_id: "home-machine-2",
      attachment_ids: [],
      worktree_id: "/repo/feature",
      status: "Parked",
    }),
  ]
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListSessions: null })
    return { SessionsListed: { sessions } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/main", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("session list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /`main` \(`session-1`\) - running - 1 CLI - main - home home-kernel-1 current/)
  assert.match(result.message ?? "", /`session-2` - parked - 0 CLIs - feature - home home-machine-2/)
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
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("prompt hello"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-1 submitted/)
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
                  tool: "arroba_read_artifact",
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

test("executeShellCommand removes shell-local variables", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    variables: { stale: "agent-1", keep: "session-1" },
  })
  const result = await executeShellCommand(parseShellCommand("unset stale"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.variableRemovals, ["stale"])
  const next = applyShellCommandResult(context, result)
  assert.deepEqual(next.variables, { keep: "session-1" })
})

test("executeShellCommand creates a session and binds assignment", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /workspace live sync: config default/)
  assert.deepEqual(result.bindings, { s: "session-2" })
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: "agent-1",
    workspace: "/repo",
    worktree: "/repo/qa",
  })
})

test("executeShellCommand does not adopt stale focused agent ids from session payloads", async () => {
  const session = makeSession({
    id: "session-2",
    worktree_id: "/repo/qa",
    focused_agent_id: "stale-agent",
    agents: [makeAgent({ id: "agent-1" })],
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const result = await executeShellCommand(parseShellCommand("session new --dir qa"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })

  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: "agent-1",
    workspace: "/repo",
    worktree: "/repo/qa",
  })
})

test("executeShellCommand reports explicit session workspace live sync mode after create", async () => {
  const session = makeSession({
    id: "session-2",
    worktree_id: "/repo/qa",
    focused_agent_id: "agent-1",
    workspace_live_sync_mode: "tracked",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /workspace live sync: tracked \(selected workspace\/worktree only; other repositories unrestricted\); use `workspace sync off` to disable/)
})

test("executeShellCommand creates and starts a headless slice for a new session", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return { SliceCreated: { slice: { id: "slice-1" } } }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-1" } } }
    }
    if ("CreateSession" in request) {
      return { SessionCreated: { session } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa --slice new as s"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "CreateSession"])
  const createRequest = requests[0] as { CreateSlice: { name: string } }
  assert.match(createRequest.CreateSlice.name, /^qa-slice-/)
  assert.deepEqual({
    ...requests[0],
    CreateSlice: {
      ...(requests[0] as { CreateSlice: Record<string, unknown> }).CreateSlice,
      name: "<dynamic>",
    },
  }, {
    CreateSlice: {
      name: "<dynamic>",
      backend: "local_docker",
      os: "linux",
      display_mode: "headless",
      workspace_id: "/repo",
      worktree_id: "/repo/qa",
      workspace_mount: "/repo/qa",
      worker_kernel_ref: null,
      display_url: null,
      provider_auth: [],
    },
  })
  assert.deepEqual(requests[1], { StartSlice: { slice_ref: "slice-1" } })
  assert.deepEqual(requests[2], { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: "slice-1" } })
  assert.deepEqual(result.bindings, { s: "session-2" })
})

test("executeShellCommand creates and starts a headed slice for a new session", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return { SliceCreated: { slice: { id: "slice-1" } } }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-1" } } }
    }
    if ("CreateSession" in request) {
      return { SessionCreated: { session } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa --slice new --slice-display headed as s"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })

  assert.equal(result.ok, true)
  assert.equal((requests[0] as { CreateSlice: { display_mode: string } }).CreateSlice.display_mode, "headed")
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "CreateSession"])
})

test("executeShellCommand reuses only slices scoped to the session worktree", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-qa",
            name: "qa",
            status: "stopped",
            workspace_id: "/repo",
            worktree_id: "/repo/qa",
          }],
        },
      }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-qa" } } }
    }
    if ("CreateSession" in request) {
      return { SessionCreated: { session } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa --slice qa"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["ListSlices", "StartSlice", "CreateSession"])
  assert.deepEqual(requests[1], { StartSlice: { slice_ref: "slice-qa" } })
  assert.deepEqual(requests[2], { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: "slice-qa" } })
})

test("executeShellCommand rejects slices from another worktree", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListSlices: null })
    return {
      SlicesListed: {
        slices: [{
          id: "slice-main",
          name: "main",
          status: "running",
          workspace_id: "/repo",
          worktree_id: "/repo/main",
        }],
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  await assert.rejects(
    () => executeShellCommand(parseShellCommand("session new --dir qa --slice main"), context, {
      client: fake.client,
      resolveExistingDirectory: async () => "/repo/qa",
    }),
    /slice main is scoped to worktree \/repo\/main, not \/repo\/qa/,
  )
})

test("executeShellCommand creates manually managed slices scoped to the current worktree", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return {
        SliceCreated: {
          slice: {
            id: "slice-manual",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "stopped",
            display_mode: "headed",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: null,
            worker_kernel_id: null,
            worker_machine_id: null,
            providers: [],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice create linux-a --headed as sl"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{
    CreateSlice: {
      name: "linux-a",
      backend: "local_docker",
      os: "linux",
      display_mode: "headed",
      workspace_id: "/repo",
      worktree_id: "/repo/feature",
      workspace_mount: "/repo/feature",
      worker_kernel_ref: null,
      display_url: null,
      provider_auth: [],
    },
  }])
  assert.deepEqual(result.bindings, { sl: "slice-manual" })
})

test("executeShellCommand renders slice doctor diagnostics", async () => {
  const fake = fakeClient((request) => {
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "unhealthy",
            display_mode: "headed",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: null,
            worker_machine_id: null,
            providers: ["codex"],
            session_ids: ["session-1"],
            agent_ids: ["agent-1"],
            provider_auth: [{
              provider: "codex",
              state: "authenticated",
              alias: "daily",
              email: "dev@example.com",
              organization_name: "Team",
              subscription_type: "pro",
            }],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice doctor linux-a"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /slice doctor linux-a id=slice-1/)
  assert.match(result.message ?? "", /fail lifecycle: unhealthy/)
  assert.match(result.message ?? "", /ok relay: shared:wss:\/\/relay.example\/slice/)
  assert.match(result.message ?? "", /fail display: headed/)
  assert.match(result.message ?? "", /ok agents: 1 attached/)
  assert.match(result.message ?? "", /ok provider CLIs: codex/)
  assert.match(result.message ?? "", /ok provider accounts: codex:daily \(dev@example.com\)\/org=Team\/plan=pro/)
  assert.match(result.message ?? "", /next: inspect slice logs/)
})

test("executeShellCommand does not infer shared slice relay authority", async () => {
  const fake = fakeClient((request) => {
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "worker-1",
            worker_machine_id: "machine-1",
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: { url: "wss://relay.example/slice" },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice status linux-a"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /relay=unknown:wss:\/\/relay.example\/slice/)
})

test("executeShellCommand renders slice account recovery hints", async () => {
  const fake = fakeClient((request) => {
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          }],
        },
      }
    }
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice list"), context, { client: fake.client })
  const doctor = await executeShellCommand(parseShellCommand("slice doctor linux-a"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /linux-a id=slice-1 status=running/)
  assert.match(result.message ?? "", /providers=codex auth=-/)
  assert.match(result.message ?? "", /next=import or login provider accounts for codex with \/slice auth import linux-a codex or \/slice auth login linux-a codex/)
  assert.equal(doctor.ok, false)
  assert.match(doctor.message ?? "", /fail provider accounts: none/)
  assert.match(doctor.message ?? "", /next: import or login provider accounts for codex/)
})

test("executeShellCommand renders concrete or placeholder slice stale-auth recovery", async () => {
  const baseSlice = {
    id: "slice-1",
    name: "linux-a",
    backend: "local_docker",
    os: "linux",
    status: "running",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/feature",
    workspace_mount: "/repo/feature",
    worker_kernel_ref: "slice:linux-a",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    session_ids: [],
    agent_ids: [],
    relay_endpoint: { url: "wss://relay.example/slice", private: false },
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const fake = fakeClient((request) => {
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [
            {
              ...baseSlice,
              id: "slice-1",
              name: "linux-a",
              providers: ["codex"],
              provider_auth: [{ provider: "codex", state: "not_configured" }],
            },
            {
              ...baseSlice,
              id: "slice-2",
              name: "linux-b",
              providers: ["codex", "opencode:openai"],
              provider_auth: [
                { provider: "codex", state: "not_configured" },
                { provider: "opencode:openai", state: "unknown" },
              ],
            },
          ],
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /linux-a[\s\S]*next=refresh provider login for codex with \/slice auth login linux-a codex/)
  assert.match(result.message ?? "", /linux-b[\s\S]*next=refresh provider login for codex,opencode:openai with \/slice auth login linux-b <provider>/)
})

test("executeShellCommand treats unsupported slice auth responses as failures", async () => {
  const slice = {
    id: "slice-1",
    name: "linux-a",
    backend: "ssh_docker",
    os: "linux",
    status: "stopped",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/feature",
    workspace_mount: "/repo/feature",
    worker_kernel_ref: "slice:linux-a",
    worker_kernel_id: null,
    worker_machine_id: null,
    providers: [],
    session_ids: [],
    agent_ids: [],
    provider_auth: [],
    relay_endpoint: null,
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const fake = fakeClient((request) => {
    if ("ImportSliceProviderAuth" in request) {
      return { SliceProviderAuthImported: { slice, provider: "codex", status: "not_implemented" } }
    }
    if ("RemoveSliceProviderAuth" in request) {
      return { SliceProviderAuthRemoved: { slice, provider: "codex", status: "not_implemented" } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })

  const imported = await executeShellCommand(parseShellCommand("slice auth import linux-a codex"), context, { client: fake.client })
  const removed = await executeShellCommand(parseShellCommand("slice auth remove linux-a codex"), context, { client: fake.client })

  assert.equal(imported.ok, false)
  assert.match(imported.message ?? "", /auth import codex: not_implemented/)
  assert.equal(removed.ok, false)
  assert.match(removed.message ?? "", /auth remove codex: not_implemented/)
})

test("executeShellCommand resolves focused agent slice by attached agent id", async () => {
  const requests: Record<string, unknown>[] = []
  const wrongWorkerSlice = {
    id: "slice-wrong",
    name: "wrong-by-worker",
    backend: "local_docker",
    os: "linux",
    status: "running",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/other",
    workspace_mount: "/repo/other",
    worker_kernel_ref: "slice:wrong-by-worker",
    worker_kernel_id: "kernel-agent",
    worker_machine_id: "machine-agent",
    providers: [],
    session_ids: ["session-1"],
    agent_ids: ["agent-other"],
    provider_auth: [],
    relay_endpoint: null,
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const slice = {
    id: "slice-1",
    name: "linux-a",
    backend: "local_docker",
    os: "linux",
    status: "running",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/feature",
    workspace_mount: "/repo/feature",
    worker_kernel_ref: "slice:linux-a",
    worker_kernel_id: "kernel-slice-other",
    worker_machine_id: "machine-slice-other",
    providers: ["codex"],
    session_ids: ["session-1"],
    agent_ids: ["agent-1"],
    provider_auth: [{
      provider: "codex",
      state: "not_configured",
      auth_type: "oauth",
    }],
    relay_endpoint: { url: "wss://relay.example/slice", private: false },
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent({ remote_execution: { worker_kernel_id: "kernel-agent", worker_machine_id: "machine-agent", execution_lease_id: "lease-1", leased_agent_id: "leased-agent-1" } })] } }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [wrongWorkerSlice, slice] } }
    }
    if ("GetSlice" in request) {
      return { Slice: { slice } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo/feature",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(parseShellCommand("slice status"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["ListAgents", "ListSlices", "GetSlice"])
  assert.deepEqual(requests[2], { GetSlice: { slice_ref: "slice-1" } })
  assert.match(result.message ?? "", /linux-a id=slice-1 status=running/)
  assert.match(result.message ?? "", /relay=shared:wss:\/\/relay.example\/slice/)
  assert.match(result.message ?? "", /auth=codex:oauth/)
  assert.match(result.message ?? "", /next=refresh provider login for codex with \/slice auth login linux-a codex/)
})

test("executeShellCommand renders slice logs", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("GetSliceLogs" in request) {
      return {
        SliceLogs: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: [],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
          entries: [{
            source: "container",
            path: null,
            text: "worker started",
            truncated: false,
          }],
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice logs linux-a --tail 50"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{ GetSliceLogs: { slice_ref: "linux-a", tail_lines: 50 } }])
  assert.match(result.message ?? "", /slice logs linux-a id=slice-1/)
  assert.match(result.message ?? "", /== container ==/)
  assert.match(result.message ?? "", /worker started/)
})

test("executeShellCommand attaches standalone shell clients when switching sessions", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateSession" in request) {
          return { SessionCreated: { session } }
        }
        return { SessionAttached: { attachment: { id: "attachment-shell" } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    clientId: "arroba-shell-test",
    resolveExistingDirectory: async () => "/repo/qa",
  })
  assert.equal(result.ok, true)
  assert.equal(result.contextUpdates?.attachmentId, "attachment-shell")
  assert.deepEqual(requests, [
    { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } },
    { AttachToSession: { session_id: "session-2", client_id: "arroba-shell-test", capability_level: "FullTerminal" } },
  ])
})

function daemonHealth(overrides: Partial<DaemonHealthProjection> = {}): DaemonHealthProjection {
  const base: DaemonHealthProjection = {
    metadata: { projection_version: 1, last_event_id: 0, generated_at_ms: 0 },
    session_command_lanes: [],
    agent_command_lanes: [],
    workflow_command_lanes: [],
    provider_runtime_lanes: [],
    provider_run_actor: { enqueued_commands: 0, enqueue_rejections: 0 },
    process: { process_id: 1234, current_resident_set_bytes: 128, peak_resident_set_bytes: 256 },
    capability_executor: {
      max_concurrent_jobs: 64,
      available_permits: 64,
      submitted_jobs: 0,
      running_jobs: 0,
      completed_jobs: 0,
      failed_jobs: 0,
      rejected_jobs: 0,
      join_errors: 0,
    },
    session_projection: {
      projected_sessions: 1,
      projected_session_list_entries: 1,
      active_prompts: 0,
      queued_prompts: 0,
    },
    agent_runtime_projection: {
      projected_agents: 1,
      active_prompts: 0,
      queued_prompts: 0,
    },
    provider_catalog: {
      cached: true,
      expired: false,
      age_ms: 1000,
      ttl_ms: 60000,
    },
    provider_runs: {
      projected_runs: 0,
      active_runs: 0,
      arroba_active_runs: 0,
      native_tui_active_runs: 0,
      duplicate_arroba_agent_bindings: [],
      multi_interface_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
    transport: {
      active_connections: 1,
      active_subscriptions: 1,
      retained_event_limit: 1000,
      command_result_cache_limit: 1000,
      inbound_request_limit: 100,
      incoming_requests: 0,
      emitted_events: 0,
      replay_gaps: 0,
      inbound_overload_rejections: 0,
      duplicate_command_conflicts: 0,
      outgoing_queue_overflows: 0,
      slow_consumer_closes: 0,
    },
    terminal_stream: {
      pending_output_records: 0,
      pending_notice_records: 0,
      pending_completion_records: 0,
      pending_output_record_limit_per_attachment: 4096,
      trimmed_pending_output_recipients: 0,
    },
    slice_lifecycle: {
      total_slices: 0,
      running_slices: 0,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 0,
      failed_operations: 0,
      in_progress_operations: 0,
      issues: [],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
    remote_execution: {
      remote_agents: 0,
      active_remote_agents: 0,
      missing_active_worker_runs: 0,
      malformed_bindings: 0,
      issues: [],
    },
    remote_extension_sync: {
      remote_agents: 0,
      home_proxy_agents: 0,
      home_proxy_grants: 0,
      manifest_missing_agents: 0,
      synced_agents: 0,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
      issues: [],
    },
    workspace_coordination: {
      active_worktree_claims: [],
      worktree_collisions: [],
      active_operation_claims: [],
    },
    workspace_live_sync: {
      active_reservations: 0,
      active_reservation_artifacts: 0,
      managed_mode: {
        write_fence_supported: true,
        write_fence_backend: "macos-seatbelt",
        unavailable_reason: null,
      },
      workspace_identity: {
        tracked_provider_runs: 0,
        identity_changed_provider_runs: 0,
        invalid_provider_runs: 0,
        current_generation_total: 0,
        issues: [],
      },
      external_changes: {
        tracked_artifacts: 0,
        externally_changed_artifacts: 0,
        external_change_events: 0,
        live_watcher_started: true,
        live_watcher_scans: 0,
        live_watcher_scan_errors: 0,
        issues: [],
      },
    },
    projection_invariants: {
      checked_sessions: 1,
      checked_agents: 1,
      mismatches: [],
    },
  }
  return { ...base, ...overrides }
}

test("executeShellCommand manages session invites and members", async () => {
  const session = makeSession({
    id: "session-1",
    owner_user_id: "local",
    members: [{ user_id: "local", joined_at_ms: 0, invited_by_user_id: null }],
    invites: [],
  })
  const invite = {
    invite_id: "invite-1",
    session_id: "session-1",
    created_by_user_id: "local",
    created_at_ms: 100,
    expires_at_ms: null,
    max_uses: 1,
    used_count: 0,
    revoked_at_ms: null,
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateSessionInvite" in request) {
          return { SessionInviteCreated: { invite: { invite, invite_token: "arroba-session-invite-v1.token" }, session } }
        }
        if ("JoinSessionInvite" in request) {
          return {
            SessionInviteJoined: {
              member: { user_id: "ana", joined_at_ms: 200, invited_by_user_id: "local" },
              session: { ...session, members: [...(session.members ?? []), { user_id: "ana", joined_at_ms: 200, invited_by_user_id: "local" }] },
            },
          }
        }
        if ("ListSessionMembers" in request) {
          return { SessionMembersListed: { members: session.members, invites: [invite] } }
        }
        if ("RevokeSessionInvite" in request) {
          return { SessionInviteRevoked: { invite: { ...invite, revoked_at_ms: 300 }, session } }
        }
        return { SessionAttached: { attachment: { id: "attachment-shell" } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const inviteResult = await executeShellCommand(parseShellCommand("session invite create"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("session join arroba-session-invite-v1.token ana"), context, { client: fake.client, clientId: "shell-ana" })
  const membersResult = await executeShellCommand(parseShellCommand("session members"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("session revoke-invite invite-1"), context, { client: fake.client })

  assert.match(inviteResult.message ?? "", /session invite invite-1/)
  assert.match(joinResult.message ?? "", /joined session session-1 as ana/)
  assert.match(membersResult.message ?? "", /Session members/)
  assert.match(revokeResult.message ?? "", /revoked session invite invite-1/)
  assert.deepEqual(requests, [
    { CreateSessionInvite: { session_id: "session-1", expires_in_ms: null, max_uses: 1, collaboration_level: "private" } },
    { JoinSessionInvite: { invite_token: "arroba-session-invite-v1.token", user_id: "ana" } },
    { AttachToSession: { session_id: "session-1", client_id: "shell-ana", capability_level: "FullTerminal" } },
    { ListSessionMembers: { session_id: "session-1" } },
    { RevokeSessionInvite: { session_id: "session-1", invite_ref: "invite-1" } },
  ])
})

test("executeShellCommand manages workspace links", async () => {
  const session = makeSession({ id: "session-1" })
  const link: WorkspaceLinkDefinition = {
    link_id: "workspace-link-1",
    session_id: "session-1",
    name: "shared-repo",
    created_by_user_id: "local",
    created_at_ms: 100,
    attachments: [],
  }
  const attached = {
    ...link,
    attachments: [{
      link_id: link.link_id,
      user_id: "local",
      machine_id: "machine-1",
      kernel_id: "kernel-1",
      repo_root: "/repo",
      branch: null,
      repo_fingerprint: null,
      attached_at_ms: 200,
    }],
  }
  const syncStatus = {
    session_id: "session-1",
    mode: "tracked",
    footer_state: "tracked",
    sync_groups: [{
      group_id: link.link_id,
      group_name: link.name,
      target_count: 1,
      ready_targets: 1,
      degraded_targets: 0,
      conflicted_targets: 0,
    }],
    targets: [{
      link_id: link.link_id,
      link_name: link.name,
      user_id: "local",
      machine_id: "machine-1",
      kernel_id: "kernel-1",
      repo_root: "/repo",
      branch: null,
      repo_fingerprint: null,
      status: "ready",
      attached_at_ms: 200,
    }],
    conflicts: [{
      conflict_id: "conflict-1",
      link_id: link.link_id,
      source_agent_id: "agent-1",
      target_user_id: "local",
      target_repo_root: "/repo",
      path: "src/app.ts",
      next_action: "reconcile target",
    }],
    ignore: { ignore_file: ".arrobaignore", rules: ["ignored/**"], force_excludes: [".git/**"] },
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateWorkspaceLink" in request) {
          return { WorkspaceLinkCreated: { link, session } }
        }
        if ("ListWorkspaceLinks" in request) {
          return { WorkspaceLinksListed: { links: [attached] } }
        }
        if ("ShowWorkspaceLink" in request) {
          return { WorkspaceLinkShown: { link: attached } }
        }
        if ("AttachWorkspaceLink" in request) {
          return { WorkspaceLinkAttached: { link: attached, attachment: attached.attachments[0], session } }
        }
        if ("DetachWorkspaceLink" in request) {
          return { WorkspaceLinkDetached: { link, detached: attached.attachments, session } }
        }
        if ("GetWorkspaceLiveSyncStatus" in request) {
          return { WorkspaceLiveSyncStatus: { status: syncStatus } }
        }
        if ("SetWorkspaceLiveSyncMode" in request) {
          return { WorkspaceLiveSyncModeUpdated: { session } }
        }
        throw new Error("unexpected request")
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })

  const createResult = await executeShellCommand(parseShellCommand("workspace link create shared-repo"), context, { client: fake.client })
  const listResult = await executeShellCommand(parseShellCommand("workspace link list"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workspace link show shared-repo"), context, { client: fake.client })
  const attachResult = await executeShellCommand(parseShellCommand("workspace link attach shared-repo"), context, { client: fake.client })
  const syncResult = await executeShellCommand(parseShellCommand("workspace sync status"), context, { client: fake.client })
  const syncTargetsResult = await executeShellCommand(parseShellCommand("workspace sync targets"), context, { client: fake.client })
  const syncConflictsResult = await executeShellCommand(parseShellCommand("workspace sync conflicts"), context, { client: fake.client })
  const syncIgnoreResult = await executeShellCommand(parseShellCommand("workspace sync ignore"), context, { client: fake.client })
  const modeResult = await executeShellCommand(parseShellCommand("workspace sync mode tracked"), context, { client: fake.client })
  const enableResult = await executeShellCommand(parseShellCommand("workspace sync enable managed"), context, { client: fake.client })
  const enableTrackedResult = await executeShellCommand(parseShellCommand("workspace sync enable tracked"), context, { client: fake.client })
  const directOffResult = await executeShellCommand(parseShellCommand("workspace sync off"), context, { client: fake.client })
  const directManagedResult = await executeShellCommand(parseShellCommand("workspace sync managed"), context, { client: fake.client })
  const disableResult = await executeShellCommand(parseShellCommand("workspace sync disable"), context, { client: fake.client })
  const syncLinkResult = await executeShellCommand(parseShellCommand("workspace sync link shared-repo"), context, { client: fake.client })
  const legacyModeOnResult = await executeShellCommand(parseShellCommand("workspace sync mode on"), context, { client: fake.client })
  const legacyModeOffResult = await executeShellCommand(parseShellCommand("workspace sync mode off"), context, { client: fake.client })
  const legacyEnableOnResult = await executeShellCommand(parseShellCommand("workspace sync enable on"), context, { client: fake.client })
  const detachResult = await executeShellCommand(parseShellCommand("workspace link detach shared-repo"), context, { client: fake.client })
  const invalidResourceResult = await executeShellCommand(parseShellCommand("workspace unknown"), context, { client: fake.client })

  assert.match(createResult.message ?? "", /created workspace link shared-repo/)
  assert.match(listResult.message ?? "", /attachments=1/)
  assert.match(showResult.message ?? "", /workspace link shared-repo/)
  assert.match(attachResult.message ?? "", /workspace sync managed.*recommended/)
  assert.match(syncResult.message ?? "", /workspace live sync: tracked/)
  assert.match(syncResult.message ?? "", /scope=selected workspace\/worktree only; other repositories are unrestricted/)
  assert.match(syncResult.message ?? "", /sync_groups=1/)
  assert.match(syncResult.message ?? "", /next=inspect workspace sync conflicts, ask an agent to reconcile, then rerun workspace sync status/)
  assert.match(syncResult.message ?? "", /group shared-repo \(workspace-link-1\) targets=1 ready=1 degraded=0 conflicts=0/)
  assert.match(syncResult.message ?? "", /source=agent-1 target=local:\/repo/)
  assert.match(syncResult.message ?? "", /rule ignored\/\*\*/)
  assert.match(syncResult.message ?? "", /force-exclude \.git\/\*\*/)
  assert.match(syncTargetsResult.message ?? "", /group shared-repo \(workspace-link-1\) targets=1 ready=1 degraded=0 conflicts=0/)
  assert.match(syncTargetsResult.message ?? "", /ready shared-repo: local \/repo/)
  assert.match(syncTargetsResult.message ?? "", /next=inspect workspace sync conflicts/)
  assert.match(syncConflictsResult.message ?? "", /src\/app\.ts source=agent-1 target=local:\/repo: reconcile target/)
  assert.match(syncIgnoreResult.message ?? "", /ignore=\.arrobaignore/)
  assert.match(syncIgnoreResult.message ?? "", /rule ignored\/\*\*/)
  assert.match(syncIgnoreResult.message ?? "", /force-exclude \.git\/\*\*/)
  assert.match(modeResult.message ?? "", /current session workspace live sync set to tracked/)
  assert.match(enableResult.message ?? "", /current session workspace live sync enabled: managed/)
  assert.match(enableTrackedResult.message ?? "", /current session workspace live sync enabled: tracked/)
  assert.match(directOffResult.message ?? "", /disabled/)
  assert.match(directManagedResult.message ?? "", /current session workspace live sync set to managed/)
  assert.match(disableResult.message ?? "", /disabled/)
  assert.match(syncLinkResult.message ?? "", /recommended mode: managed/)
  assert.match(legacyModeOnResult.message ?? "", /usage: workspace sync mode off\|managed\|tracked/)
  assert.match(legacyModeOffResult.message ?? "", /disabled/)
  assert.match(legacyEnableOnResult.message ?? "", /usage: workspace sync enable \[managed\|tracked\]/)
  assert.match(detachResult.message ?? "", /detached 1 workspace link attachment/)
  assert.match(invalidResourceResult.message ?? "", /workspace sync .*link/)
  assert.deepEqual(requests, [
    { CreateWorkspaceLink: { session_id: "session-1", name: "shared-repo" } },
    { ListWorkspaceLinks: { session_id: "session-1" } },
    { ShowWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo" } },
    { AttachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo", branch: null, repo_fingerprint: null } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "tracked" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "managed" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "tracked" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "unrestricted" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "managed" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "unrestricted" } },
    { AttachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo", branch: null, repo_fingerprint: null } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "unrestricted" } },
    { DetachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo" } },
  ])
})
