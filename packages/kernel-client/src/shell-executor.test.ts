import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  ProviderProcessInfo,
  RuntimeSession,
  WorkflowWatchdogDefinition,
  WorkflowDefinition,
  WorkflowPublicationDefinition,
  WorkflowPublicationTrustedSender,
  WorkflowRun,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"

function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.2",
    worktree_id: "/repo",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}

function makeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/repo",
    worktree_id: "/repo",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [makeAgent()],
    config_state: { version: 0, values: {} },
    ...overrides,
  }
}

function makeWorkflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: "qa",
    flush_agent_context_before_run: true,
    nodes: [{ id: "node-1", agent_id: "agent-1" }],
    edges: [],
    endpoints: [{ id: "endpoint-1", alias: "default", entry_node_id: "node-1" }],
    ...overrides,
  }
}

function makeWorkflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status: "Running",
    invocation_prompt: "Run QA",
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 0,
    started_at_ms: 0,
    completed_at_ms: null,
    ...overrides,
  }
}

function makeWorkflowPublication(overrides: Partial<WorkflowPublicationDefinition> = {}): WorkflowPublicationDefinition {
  return {
    id: "publication-1",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "public_qa",
    enabled: true,
    route: "/qa",
    methods: ["POST"],
    auth: { mode: "anonymous" },
    parser: { kind: "json" },
    created_by_user_id: "local",
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function makeWorkflowWatchdog(overrides: Partial<WorkflowWatchdogDefinition> = {}): WorkflowWatchdogDefinition {
  return {
    id: "watchdog-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    enabled: true,
    interval_seconds: 60,
    invocation_prompt: "Run it",
    policy: "skip",
    wakeups_executed: 0,
    next_run_at_ms: 0,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function fakeClient(handler: (request: Record<string, unknown>) => Record<string, unknown>) {
  const requests: Record<string, unknown>[] = []
  return {
    requests,
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return handler(request)
      },
    },
  }
}

test("executeShellCommand handles shell-local context mutations", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("set model gpt-5.3"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, { model: "gpt-5.3" })
  const next = applyShellCommandResult(context, result)
  assert.equal(next.model, "gpt-5.3")
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
        },
      }
    }
    return {}
  })
  const contextResult = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })
  const pwdResult = await executeShellCommand(parseShellCommand("pwd"), context, { client: fake.client })

  assert.equal(contextResult.ok, true)
  assert.match(contextResult.message ?? "", /workspace: \/repo/)
  assert.match(contextResult.message ?? "", /worktree: \/repo\/worktree/)
  assert.match(contextResult.message ?? "", /session: session-1/)
  assert.match(contextResult.message ?? "", /agent: agent-1 \(busy\)/)
  assert.match(contextResult.message ?? "", /workflow: workflow-1/)
  assert.match(contextResult.message ?? "", /provider: codex/)
  assert.match(contextResult.message ?? "", /\$wf = workflow-1/)
  assert.equal(pwdResult.message, "/repo/worktree")
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
    if ("GetSessionHistory" in request) {
      return {
        SessionHistory: {
          next_cursor: null,
          entries: [
            {
              entry_index: 1,
              fragment_start: 0,
              fragment_end: 5,
              total_chars: 5,
              entry: { agent_id: "agent-1", kind: "user_prompt", text: "hello\n" },
            },
            {
              entry_index: 2,
              fragment_start: 0,
              fragment_end: 7,
              total_chars: 7,
              entry: { agent_id: "agent-1", kind: "provider_output", text: "done ok" },
            },
          ],
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
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { s: "session-2" })
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: "agent-1",
    workspace: "/repo",
    worktree: "/repo/qa",
  })
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
    { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null } },
    { AttachToSession: { session_id: "session-2", client_id: "arroba-shell-test", capability_level: "FullTerminal" } },
  ])
})

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
    { CreateSessionInvite: { session_id: "session-1", expires_in_ms: null, max_uses: 1 } },
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
        throw new Error("unexpected request")
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })

  const createResult = await executeShellCommand(parseShellCommand("workspace link create shared-repo"), context, { client: fake.client })
  const listResult = await executeShellCommand(parseShellCommand("workspace link list"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workspace link show shared-repo"), context, { client: fake.client })
  const attachResult = await executeShellCommand(parseShellCommand("workspace link attach shared-repo"), context, { client: fake.client })
  const detachResult = await executeShellCommand(parseShellCommand("workspace link detach shared-repo"), context, { client: fake.client })

  assert.match(createResult.message ?? "", /created workspace link shared-repo/)
  assert.match(listResult.message ?? "", /attachments=1/)
  assert.match(showResult.message ?? "", /workspace link shared-repo/)
  assert.match(attachResult.message ?? "", /attached \/repo/)
  assert.match(detachResult.message ?? "", /detached 1 workspace link attachment/)
  assert.deepEqual(requests, [
    { CreateWorkspaceLink: { session_id: "session-1", name: "shared-repo" } },
    { ListWorkspaceLinks: { session_id: "session-1" } },
    { ShowWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo" } },
    { AttachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo", branch: null, repo_fingerprint: null } },
    { DetachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo" } },
  ])
})

test("executeShellCommand lists agents for current session", async () => {
  const agents = [makeAgent(), makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })]
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListAgents: { session_id: "session-1" } })
    return { AgentsListed: { agents } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /2 agents/)
  assert.deepEqual((result.data as { agents: AgentInstance[] }).agents, agents)
})

test("executeShellCommand spawns remote agent with worktree placement", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "qa",
    worktree_id: "/remote/qa",
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "mac-mini",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, {
      SpawnAgent: {
        session_id: "session-1",
        provider: "codex",
        alias: "qa",
        model: "gpt-5.2",
        effort: "low",
        worktree_id: "/remote/qa",
        machine_ref: "mac-mini",
        worktree_placement: null,
      },
    })
    return { AgentSpawned: { agent } }
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })
  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine mac-mini --dir /remote/qa as qa"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { qa: "agent-remote" })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-remote" })
})

test("executeShellCommand rejects agent commands without current session", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /no current session/)
})

test("executeShellCommand lists remote machines", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListRemoteMachines: null })
    return {
      RemoteMachinesListed: {
        machines: [{
          machine_id: "machine-1",
          machine_alias: "mini",
          registry_alias: null,
          display_name: "mini",
          trust_status: "approved",
          online: true,
          pending: false,
          kernel_count: 1,
          available_providers: ["codex"],
        }],
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("machine list"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /mini id=machine-1/)
})

test("executeShellCommand manages remote machine trust", async () => {
  const machine = {
    machine_id: "machine-1",
    machine_alias: "mini",
    registry_alias: "mini",
    display_name: "mini",
    trust_status: "approved",
    online: true,
    pending: false,
    kernel_count: 1,
    available_providers: ["codex"],
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ApproveRemoteMachine" in request) {
          return { RemoteMachineApproved: { machine } }
        }
        if ("RenameRemoteMachine" in request) {
          return { RemoteMachineRenamed: { machine: { ...machine, registry_alias: "builder" } } }
        }
        if ("ForgetRemoteMachine" in request) {
          return { RemoteMachineForgotten: { machine: { ...machine, trust_status: "forgotten" } } }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const approveResult = await executeShellCommand(parseShellCommand("machine approve machine-1"), context, { client: fake.client })
  const renameResult = await executeShellCommand(parseShellCommand("machine rename machine-1 builder"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("machine revoke machine-1"), context, { client: fake.client })

  assert.equal(approveResult.ok, true)
  assert.match(approveResult.message ?? "", /approved machine mini/)
  assert.equal(renameResult.ok, true)
  assert.match(renameResult.message ?? "", /renamed machine mini/)
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked machine mini/)
  assert.deepEqual(requests, [
    { ApproveRemoteMachine: { machine_ref: "machine-1" } },
    { RenameRemoteMachine: { machine_ref: "machine-1", alias: "builder" } },
    { ForgetRemoteMachine: { machine_ref: "machine-1" } },
  ])
})

test("executeShellCommand creates and joins machine invites", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreatePairingInvite" in request) {
          return {
            PairingInviteCreated: {
              invite: {
                intent: "machine",
                invite_id: "invite-1",
                invite_token: "arroba-invite-v1.machine",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                target_daemon_alias: null,
                issued_at_ms: 1,
                expires_at_ms: 2,
              },
            },
          }
        }
        if ("JoinPairingInvite" in request) {
          return {
            PairingInviteJoined: {
              pairing: {
                intent: "machine",
                subject_id: "machine-2",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                alias: "worker",
                public_key_thumbprint: "thumbprint-2",
                paired_at_ms: 3,
              },
            },
          }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const inviteResult = await executeShellCommand(parseShellCommand("machine invite create worker"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("machine join arroba-invite-v1.machine machine-2 worker"), context, { client: fake.client })

  assert.equal(inviteResult.ok, true)
  assert.match(inviteResult.message ?? "", /machine invite invite-1/)
  assert.match(inviteResult.message ?? "", /token=arroba-invite-v1\.machine/)
  assert.equal(joinResult.ok, true)
  assert.match(joinResult.message ?? "", /joined machine machine-2 alias=worker/)
  assert.deepEqual(requests, [
    { CreatePairingInvite: { intent: "machine", alias: "worker", expires_in_ms: null } },
    {
      JoinPairingInvite: {
        invite_token: "arroba-invite-v1.machine",
        subject_id: "machine-2",
        public_key_thumbprint: null,
        alias: "worker",
      },
    },
  ])
})

test("executeShellCommand manages paired clients", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListPairedClients" in request) {
          return {
            PairedClientsListed: {
              clients: [{
                client_id: "client-1",
                alias: "desk",
                public_key_thumbprint: "thumbprint-1",
                paired_at_ms: 42,
                revoked: false,
              }],
            },
          }
        }
        if ("RecordPairedClient" in request) {
          return {
            PairedClientRecorded: {
              client: {
                client_id: "client-2",
                alias: "laptop",
                public_key_thumbprint: "thumbprint-2",
                paired_at_ms: 84,
                revoked: false,
              },
            },
          }
        }
        if ("RevokePairedClient" in request) {
          return {
            PairedClientRevoked: {
              client: {
                client_id: "client-2",
                alias: "laptop",
                public_key_thumbprint: "thumbprint-2",
                paired_at_ms: 84,
                revoked: true,
              },
            },
          }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const listResult = await executeShellCommand(parseShellCommand("client list"), context, { client: fake.client })
  const recordResult = await executeShellCommand(parseShellCommand("client record client-2 thumbprint-2 laptop"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("client revoke client-2"), context, { client: fake.client })

  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /desk id=client-1 thumbprint=thumbprint-1 paired_at_ms=42/)
  assert.equal(recordResult.ok, true)
  assert.match(recordResult.message ?? "", /paired client laptop id=client-2/)
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked client laptop id=client-2/)
  assert.deepEqual(requests, [
    { ListPairedClients: null },
    {
      RecordPairedClient: {
        client_id: "client-2",
        public_key_thumbprint: "thumbprint-2",
        alias: "laptop",
        paired_at_ms: null,
      },
    },
    { RevokePairedClient: { client_id: "client-2" } },
  ])
})

test("executeShellCommand creates and joins client invites", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreatePairingInvite" in request) {
          return {
            PairingInviteCreated: {
              invite: {
                intent: "client",
                invite_id: "invite-client",
                invite_token: "arroba-invite-v1.client",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                target_daemon_alias: "home",
                issued_at_ms: 1,
                expires_at_ms: 2,
              },
            },
          }
        }
        if ("JoinPairingInvite" in request) {
          return {
            PairingInviteJoined: {
              pairing: {
                intent: "client",
                subject_id: "client-2",
                relay_url: "ws://relay",
                target_daemon_id: "daemon-1",
                alias: "desk",
                public_key_thumbprint: "thumbprint-client",
                paired_at_ms: 3,
              },
            },
          }
        }
        return {}
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const inviteResult = await executeShellCommand(parseShellCommand("client invite create desk"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("client join arroba-invite-v1.client client-2 desk"), context, { client: fake.client })

  assert.equal(inviteResult.ok, true)
  assert.match(inviteResult.message ?? "", /client invite invite-client/)
  assert.equal(joinResult.ok, true)
  assert.match(joinResult.message ?? "", /joined client client-2 alias=desk/)
  assert.deepEqual(requests, [
    { CreatePairingInvite: { intent: "client", alias: "desk", expires_in_ms: null } },
    {
      JoinPairingInvite: {
        invite_token: "arroba-invite-v1.client",
        subject_id: "client-2",
        public_key_thumbprint: null,
        alias: "desk",
      },
    },
  ])
})

test("executeShellCommand reports relay status", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { RelayStatus: null })
    return {
      RelayStatus: {
        status: {
          configured: true,
          connected: false,
          relay_url: "wss://relay.example",
          relay_token_configured: true,
          daemon_id: "daemon-1",
          machine_id: "machine-1",
          machine_alias: "mini",
        },
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("relay status"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /configured, disconnected/)
  assert.match(result.message ?? "", /machine=mini/)
})

test("executeShellCommand lists MCP servers and skills in the workspace", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListMcpServers" in request) {
          return { McpServersListed: { mcps: [{ name: "playwright", transport: { stdio: { command: "npx" } }, enabled: true }] } }
        }
        return { SkillsListed: { skills: [{ name: "qa", description: "QA checks", path: "/skills/qa" }] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const mcpResult = await executeShellCommand(parseShellCommand("mcp list"), context, { client: fake.client })
  const skillResult = await executeShellCommand(parseShellCommand("skill list"), context, { client: fake.client })
  assert.equal(mcpResult.ok, true)
  assert.match(mcpResult.message ?? "", /playwright \[enabled\]/)
  assert.equal(skillResult.ok, true)
  assert.match(skillResult.message ?? "", /qa - QA checks/)
  assert.deepEqual(requests, [
    { ListMcpServers: { workspace_id: "/repo" } },
    { ListSkills: { workspace_id: "/repo" } },
  ])
})

test("executeShellCommand shows config and provider auth status", async () => {
  const fake = fakeClient((request) => {
    if ("GetUserConfig" in request) {
      return { UserConfig: { path: "/home/.arroba/config.json", config: { version: 1, providers: { default: "codex" } } } }
    }
    assert.deepEqual(request, { GetProviderAuthStatus: { provider: "codex" } })
    return {
      ProviderAuthStatus: {
        status: {
          provider: "codex",
          auth_state: "authenticated",
          account_profile: "default",
          login_hint: null,
          detected_version: "1.2.3",
        },
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", provider: "codex" })
  const configResult = await executeShellCommand(parseShellCommand("config show"), context, { client: fake.client })
  const providerResult = await executeShellCommand(parseShellCommand("provider status"), context, { client: fake.client })
  assert.equal(configResult.ok, true)
  assert.match(configResult.message ?? "", /"default": "codex"/)
  assert.equal(providerResult.ok, true)
  assert.match(providerResult.message ?? "", /codex: authenticated as default/)
  assert.match(providerResult.message ?? "", /version 1.2.3/)
})

test("executeShellCommand mutates user config", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetUserConfig" in request) {
          return { UserConfig: { path: "/home/.arroba/config.json", config: { version: 1, providers: { default: "codex" } } } }
        }
        return {
          UserConfigUpdated: {
            path: "/home/.arroba/config.json",
            config: { version: 1, providers: { managed_io: { codex: "required" } } },
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const pathResult = await executeShellCommand(parseShellCommand("config path"), context, { client: fake.client })
  const setResult = await executeShellCommand(parseShellCommand("config set providers.default opencode"), context, { client: fake.client })
  const unsetResult = await executeShellCommand(parseShellCommand("config unset providers.default"), context, { client: fake.client })
  const managedIoResult = await executeShellCommand(parseShellCommand("config managed-io codex on"), context, { client: fake.client })
  assert.equal(pathResult.ok, true)
  assert.equal(pathResult.message, "/home/.arroba/config.json")
  assert.equal(setResult.ok, true)
  assert.match(setResult.message ?? "", /config providers.default set to opencode/)
  assert.equal(unsetResult.ok, true)
  assert.match(unsetResult.message ?? "", /config providers.default unset/)
  assert.equal(managedIoResult.ok, true)
  assert.match(managedIoResult.message ?? "", /managed I\/O for codex set to required/)
  assert.deepEqual(requests, [
    { GetUserConfig: null },
    { SetUserConfigValue: { path: "providers.default", value: "opencode" } },
    { UnsetUserConfigValue: { path: "providers.default" } },
    { SetUserConfigValue: { path: "providers.managed_io.codex", value: "required" } },
  ])
})

test("executeShellCommand installs and updates MCP servers", async () => {
  const installed: ArrobaMcpServerConfig = {
    name: "playwright",
    transport: { type: "stdio", command: "npx", args: ["@playwright/mcp"], env: {}, env_vars: ["GITHUB_TOKEN"] },
    enabled: true,
    required: false,
  }
  const updated: ArrobaMcpServerConfig = {
    name: "browser",
    transport: {
      type: "streamable_http",
      url: "https://mcp.example",
      bearer_token_env_var: "MCP_TOKEN",
      http_headers: {},
      env_http_headers: {},
    },
    enabled: true,
    required: false,
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InstallMcpServer" in request) {
          return { McpServerInstalled: { mcp: installed } }
        }
        return { McpServerUpdated: { mcp: updated } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const installResult = await executeShellCommand(
    parseShellCommand("mcp install playwright --command npx --arg @playwright/mcp --env GITHUB_TOKEN"),
    context,
    { client: fake.client },
  )
  const updateResult = await executeShellCommand(
    parseShellCommand("mcp update browser --url https://mcp.example --bearer-token-env-var MCP_TOKEN"),
    context,
    { client: fake.client },
  )
  assert.equal(installResult.ok, true)
  assert.match(installResult.message ?? "", /installed MCP playwright/)
  assert.equal(updateResult.ok, true)
  assert.match(updateResult.message ?? "", /updated MCP browser/)
  assert.deepEqual(requests, [
    {
      InstallMcpServer: {
        workspace_id: "/repo",
        config: installed,
      },
    },
    {
      UpdateMcpServer: {
        workspace_id: "/repo",
        config: updated,
      },
    },
  ])
})

test("executeShellCommand imports MCP servers and skills", async () => {
  const mcp: ArrobaMcpServerConfig = {
    name: "github",
    transport: { type: "stdio", command: "github-mcp-server", args: [], env: {}, env_vars: [] },
    enabled: true,
    required: false,
  }
  const skill: ArrobaSkillMetadata = {
    name: "qa",
    description: "QA checks",
    short_description: "QA",
    path: "/skills/qa",
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ImportMcpServers" in request) {
          return {
            McpServersImported: {
              outcome: {
                imported: [mcp],
                skipped: [{ name: "oauth", reason: "oauth transport is provider-native" }],
              },
            },
          }
        }
        return {
          SkillsImported: {
            outcome: {
              imported: [skill],
              skipped: [{ name: "old", reason: "already installed" }],
            },
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const mcpResult = await executeShellCommand(parseShellCommand("mcp import codex github"), context, { client: fake.client })
  const skillResult = await executeShellCommand(parseShellCommand("skill import codex qa"), context, { client: fake.client })
  assert.equal(mcpResult.ok, true)
  assert.match(mcpResult.message ?? "", /Imported MCPs: github/)
  assert.match(mcpResult.message ?? "", /oauth: oauth transport is provider-native/)
  assert.equal(skillResult.ok, true)
  assert.match(skillResult.message ?? "", /Imported skills: qa/)
  assert.match(skillResult.message ?? "", /old: already installed/)
  assert.deepEqual(requests, [
    { ImportMcpServers: { workspace_id: "/repo", provider: "codex", name: "github" } },
    { ImportSkills: { workspace_id: "/repo", provider: "codex", name: "qa" } },
  ])
})

test("executeShellCommand grants, revokes, and lists agent capabilities", async () => {
  const agent = makeAgent({ mcp_grants: ["playwright"], skill_grants: ["qa"] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GrantAgentCapability" in request) {
          return { AgentCapabilityGranted: { agent } }
        }
        if ("RevokeAgentCapability" in request) {
          return { AgentCapabilityRevoked: { agent } }
        }
        return { AgentsListed: { agents: [agent] } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const grantResult = await executeShellCommand(parseShellCommand("mcp grant agent-1 playwright"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("skill revoke agent-1 qa"), context, { client: fake.client })
  const grantsResult = await executeShellCommand(parseShellCommand("mcp grants"), context, { client: fake.client })
  assert.equal(grantResult.ok, true)
  assert.match(grantResult.message ?? "", /granted MCP playwright to agent-1/)
  assert.deepEqual(grantResult.contextUpdates, { agentId: "agent-1" })
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked skill qa from agent-1/)
  assert.equal(grantsResult.ok, true)
  assert.match(grantsResult.message ?? "", /agent-1 MCP grants/)
  assert.match(grantsResult.message ?? "", /playwright/)
  assert.deepEqual(requests, [
    { GrantAgentCapability: { workspace_id: "/repo", agent_ref: "agent-1", kind: "mcp", name: "playwright" } },
    { RevokeAgentCapability: { agent_ref: "agent-1", kind: "skill", name: "qa" } },
    { ListAgents: { session_id: "session-1" } },
  ])
})

test("executeShellCommand installs and uninstalls skills", async () => {
  const skill: ArrobaSkillMetadata = {
    name: "qa",
    description: "QA checks",
    short_description: "QA",
    path: "/skills/qa",
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InstallSkill" in request) {
          return { SkillInstalled: { skill } }
        }
        return { SkillUninstalled: { skill } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const installResult = await executeShellCommand(parseShellCommand("skill install /tmp/skills/qa"), context, { client: fake.client })
  const uninstallResult = await executeShellCommand(parseShellCommand("skill uninstall qa"), context, { client: fake.client })
  assert.equal(installResult.ok, true)
  assert.match(installResult.message ?? "", /installed skill qa/)
  assert.equal(uninstallResult.ok, true)
  assert.match(uninstallResult.message ?? "", /uninstalled skill qa/)
  assert.deepEqual(requests, [
    { InstallSkill: { workspace_id: "/repo", source_path: "/tmp/skills/qa" } },
    { UninstallSkill: { workspace_id: "/repo", name: "qa" } },
  ])
})

test("executeShellCommand manages workflow list, create, show, and alias", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [workflow] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListWorkflows" in request) {
          return { WorkflowsListed: { workflows: [workflow] } }
        }
        if ("CreateWorkflow" in request) {
          return { WorkflowCreated: { workflow, session } }
        }
        if ("ResolveWorkflow" in request) {
          return { WorkflowResolved: { workflow } }
        }
        return { WorkflowAliased: { workflow: { ...workflow, alias: "review" }, session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const listResult = await executeShellCommand(parseShellCommand("workflow list"), context, { client: fake.client })
  const newResult = await executeShellCommand(parseShellCommand("workflow new qa as wf"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workflow show workflow-1"), context, { client: fake.client })
  const aliasResult = await executeShellCommand(parseShellCommand("workflow alias workflow-1 review"), context, { client: fake.client })
  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /workflow-1 \(qa\) nodes=1/)
  assert.equal(newResult.ok, true)
  assert.deepEqual(newResult.bindings, { wf: "workflow-1" })
  assert.deepEqual(newResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
  assert.equal(showResult.ok, true)
  assert.match(showResult.message ?? "", /workflow workflow-1 \(qa\)/)
  assert.deepEqual(showResult.contextUpdates, { workflowId: "workflow-1" })
  assert.equal(aliasResult.ok, true)
  assert.match(aliasResult.message ?? "", /aliased as review/)
  assert.deepEqual(requests, [
    { ListWorkflows: { session_id: "session-1" } },
    { CreateWorkflow: { session_id: "session-1", alias: "qa" } },
    { ResolveWorkflow: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { AliasWorkflow: { session_id: "session-1", workflow_ref: "workflow-1", alias: "review" } },
  ])
})

test("executeShellCommand runs and controls workflow runs", async () => {
  const workflow = makeWorkflow()
  const workflowRun = makeWorkflowRun()
  const session = makeSession({ workflows: [workflow], workflow_runs: [workflowRun] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InvokeWorkflowEndpoint" in request) {
          return { WorkflowRunInvoked: { workflow_run: workflowRun, workflow, endpoint: workflow.endpoints![0], session } }
        }
        if ("ListWorkflowRuns" in request) {
          return { WorkflowRunsListed: { workflow_runs: [workflowRun] } }
        }
        if ("GetWorkflowRun" in request) {
          return { WorkflowRun: { workflow_run: workflowRun } }
        }
        if ("CancelWorkflowRun" in request) {
          return { WorkflowRunCancelled: { workflow_run: { ...workflowRun, status: "Cancelled" }, session } }
        }
        return { WorkflowRunResumed: { workflow_run: { ...workflowRun, status: "Running" }, session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const runResult = await executeShellCommand(parseShellCommand("workflow run workflow-1 endpoint-1 Run QA"), context, { client: fake.client })
  const runsResult = await executeShellCommand(parseShellCommand("workflow runs workflow-1"), context, { client: fake.client })
  const showRunResult = await executeShellCommand(parseShellCommand("workflow run-show run-1"), context, { client: fake.client })
  const cancelResult = await executeShellCommand(parseShellCommand("workflow cancel run-1"), context, { client: fake.client })
  const resumeResult = await executeShellCommand(parseShellCommand("workflow resume run-1"), context, { client: fake.client })
  assert.equal(runResult.ok, true)
  assert.match(runResult.message ?? "", /started workflow run run-1/)
  assert.deepEqual(runResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
  assert.equal(runsResult.ok, true)
  assert.match(runsResult.message ?? "", /run-1 workflow=workflow-1/)
  assert.equal(showRunResult.ok, true)
  assert.equal(showRunResult.format, "json")
  assert.equal(cancelResult.ok, true)
  assert.match(cancelResult.message ?? "", /cancelled workflow run run-1 \[cancelled\]/)
  assert.equal(resumeResult.ok, true)
  assert.match(resumeResult.message ?? "", /resumed workflow run run-1 \[running\]/)
  assert.deepEqual(requests, [
    { InvokeWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", prompt: "Run QA" } },
    { ListWorkflowRuns: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { GetWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
    { CancelWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
    { ResumeWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
  ])
})

test("executeShellCommand manages workflow graph and endpoints", async () => {
  const workflow = makeWorkflow({
    nodes: [
      { id: "node-1", agent_id: "agent-1" },
      { id: "node-2", agent_id: "agent-2" },
    ],
    edges: [{ id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }],
  })
  const session = makeSession({ workflows: [workflow] })
  const node = { id: "node-2", agent_id: "agent-2" }
  const edge = { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }
  const endpoint = { id: "endpoint-1", alias: "default", entry_node_id: "node-1" }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListAgents" in request) {
          return { AgentsListed: { agents: [makeAgent(), makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })] } }
        }
        if ("AddWorkflowNode" in request) {
          return { WorkflowNodeAdded: { node, workflow, session } }
        }
        if ("RemoveWorkflowNode" in request) {
          return { WorkflowNodeRemoved: { node, workflow, session } }
        }
        if ("AddWorkflowEdge" in request) {
          return { WorkflowEdgeAdded: { edge, workflow, session } }
        }
        if ("RemoveWorkflowEdge" in request) {
          return { WorkflowEdgeRemoved: { edge, workflow, session } }
        }
        if ("CreateWorkflowEndpoint" in request) {
          return { WorkflowEndpointCreated: { endpoint, workflow, session } }
        }
        if ("AliasWorkflowEndpoint" in request) {
          return { WorkflowEndpointAliased: { endpoint: { ...endpoint, alias: "smoke" }, workflow, session } }
        }
        return { WorkflowEndpointBound: { endpoint, workflow, session } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    workflowId: "workflow-1",
  })
  const nodeAdd = await executeShellCommand(parseShellCommand("workflow node add reviewer as node"), context, { client: fake.client })
  const nodeRemove = await executeShellCommand(parseShellCommand("workflow node remove node-2"), context, { client: fake.client })
  const edgeAdd = await executeShellCommand(parseShellCommand("workflow edge add node-1 node-2"), context, { client: fake.client })
  const edgeRemove = await executeShellCommand(parseShellCommand("workflow edge remove edge-1"), context, { client: fake.client })
  const endpointNew = await executeShellCommand(parseShellCommand("workflow endpoint new workflow-1 node-1 default"), context, { client: fake.client })
  const endpointAlias = await executeShellCommand(parseShellCommand("workflow endpoint alias endpoint-1 smoke"), context, { client: fake.client })
  const endpointBind = await executeShellCommand(parseShellCommand("workflow endpoint bind endpoint-1 node-1"), context, { client: fake.client })
  assert.equal(nodeAdd.ok, true)
  assert.deepEqual(nodeAdd.bindings, { node: "node-2" })
  assert.equal(nodeRemove.ok, true)
  assert.equal(edgeAdd.ok, true)
  assert.equal(edgeRemove.ok, true)
  assert.equal(endpointNew.ok, true)
  assert.equal(endpointAlias.ok, true)
  assert.equal(endpointBind.ok, true)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { AddWorkflowNode: { session_id: "session-1", workflow_ref: "workflow-1", agent_id: "agent-2" } },
    { RemoveWorkflowNode: { session_id: "session-1", workflow_ref: "workflow-1", node_id: "node-2" } },
    { AddWorkflowEdge: { session_id: "session-1", workflow_ref: "workflow-1", from_node_id: "node-1", to_node_id: "node-2" } },
    { RemoveWorkflowEdge: { session_id: "session-1", workflow_ref: "workflow-1", edge_id: "edge-1" } },
    { CreateWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", entry_node_id: "node-1", alias: "default" } },
    { AliasWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", alias: "smoke" } },
    { BindWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", entry_node_id: "node-1" } },
  ])
})

test("executeShellCommand manages workflow publications", async () => {
  const publication = makeWorkflowPublication()
  const sender: WorkflowPublicationTrustedSender = {
    sender_id: "sender-1",
    publication_id: publication.id,
    display_name: "partner",
    credential_hash: "hash",
    allowed_transports: ["http"],
    created_at_ms: 0,
  }
  const session = makeSession({ workflows: [makeWorkflow()], workflow_publications: [publication] })
  const fake = fakeClient((request) => {
    if ("CreateWorkflowPublication" in request) {
      return { WorkflowPublicationCreated: { publication, session } }
    }
    if ("CreateWorkflowPublicationPairCode" in request) {
      return {
        WorkflowPublicationPairCodeCreated: {
          pair_code: {
            code: {
              code_id: "pair-1",
              publication_id: publication.id,
              pair_code_hash: "hash",
              created_by_user_id: "local",
              created_at_ms: 0,
              used_count: 0,
            },
            pair_code: "pair-code",
          },
          session,
        },
      }
    }
    if ("RedeemWorkflowPublicationPairCode" in request) {
      return { WorkflowPublicationSenderPaired: { sender_credential: { sender, credential: "sender-secret" }, session } }
    }
    if ("ListWorkflowPublicationSenders" in request) {
      return { WorkflowPublicationSendersListed: { senders: [sender] } }
    }
    if ("RevokeWorkflowPublicationSender" in request) {
      return { WorkflowPublicationSenderRevoked: { sender: { ...sender, revoked_at_ms: 42 }, session } }
    }
    if ("ListWorkflowPublications" in request) {
      return { WorkflowPublicationsListed: { publications: [publication] } }
    }
    if ("GetWorkflowPublication" in request) {
      return { WorkflowPublication: { publication } }
    }
    return { WorkflowPublicationDisabled: { publication: { ...publication, enabled: false }, session } }
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    workflowId: "workflow-1",
  })

  const createResult = await executeShellCommand(
    parseShellCommand("workflow publication create endpoint-1 public_qa --route /qa --method POST --auth-json '{\"mode\":\"anonymous\"}'"),
    context,
    { client: fake.client },
  )
  const listResult = await executeShellCommand(parseShellCommand("workflow publication list"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workflow publication show publication-1"), context, { client: fake.client })
  const disableResult = await executeShellCommand(parseShellCommand("workflow publication disable publication-1"), context, { client: fake.client })
  const pairCodeResult = await executeShellCommand(parseShellCommand("workflow publication pair-code publication-1 --max-uses 1"), context, { client: fake.client })
  const redeemResult = await executeShellCommand(parseShellCommand("workflow publication redeem-code publication-1 pair-code partner"), context, { client: fake.client })
  const sendersResult = await executeShellCommand(parseShellCommand("workflow publication senders publication-1"), context, { client: fake.client })
  const revokeSenderResult = await executeShellCommand(parseShellCommand("workflow publication revoke-sender publication-1 sender-1"), context, { client: fake.client })

  assert.equal(createResult.ok, true)
  assert.match(createResult.message ?? "", /created workflow publication publication-1/)
  assert.deepEqual(createResult.contextUpdates, { sessionId: "session-1", agentId: "agent-1", workflowId: "workflow-1" })
  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /publication-1 \(public_qa\) workflow=workflow-1 endpoint=endpoint-1 enabled=true route=\/qa methods=POST/)
  assert.equal(showResult.ok, true)
  assert.equal(showResult.format, "json")
  assert.equal(disableResult.ok, true)
  assert.match(disableResult.message ?? "", /disabled workflow publication publication-1/)
  assert.equal(pairCodeResult.ok, true)
  assert.match(pairCodeResult.message ?? "", /pair-code/)
  assert.equal(redeemResult.ok, true)
  assert.match(redeemResult.message ?? "", /sender-secret/)
  assert.equal(sendersResult.ok, true)
  assert.match(sendersResult.message ?? "", /sender-1 \(partner\)/)
  assert.equal(revokeSenderResult.ok, true)
  assert.match(revokeSenderResult.message ?? "", /revoked workflow publication sender sender-1/)
  assert.deepEqual(fake.requests, [
    {
      CreateWorkflowPublication: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
        endpoint_ref: "endpoint-1",
        alias: "public_qa",
        route: "/qa",
        methods: ["POST"],
        transport: null,
        auth: { mode: "anonymous" },
        parser: null,
        input_schema: null,
        mode: null,
      },
    },
    { ListWorkflowPublications: { session_id: "session-1" } },
    { GetWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    { DisableWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    { CreateWorkflowPublicationPairCode: { session_id: "session-1", publication_ref: "publication-1", expires_in_ms: null, max_uses: 1 } },
    {
      RedeemWorkflowPublicationPairCode: {
        session_id: "session-1",
        publication_ref: "publication-1",
        pair_code: "pair-code",
        display_name: "partner",
        allowed_transports: ["http"],
        expires_in_ms: null,
      },
    },
    { ListWorkflowPublicationSenders: { session_id: "session-1", publication_ref: "publication-1" } },
    { RevokeWorkflowPublicationSender: { session_id: "session-1", publication_ref: "publication-1", sender_ref: "sender-1" } },
  ])
})

test("executeShellCommand exports a workflow publication gateway package", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-export-test-"))
  try {
    const publication = makeWorkflowPublication({
      auth: { mode: "arroba", paired_senders: { enabled: true } },
      mode: "async",
    })
    const fake = fakeClient((request) => {
      if ("GetWorkflowPublication" in request) {
        return { WorkflowPublication: { publication } }
      }
      throw new Error(`unexpected request ${JSON.stringify(request)}`)
    })
    const context = createDefaultShellContext({
      workspace: root,
      worktree: root,
      sessionId: "session-1",
      workflowId: "workflow-1",
    })

    const result = await executeShellCommand(
      parseShellCommand("workflow publication export publication-1 exported --kernel-url ws://kernel.example"),
      context,
      { client: fake.client },
    )

    assert.equal(result.ok, true)
    assert.match(result.message ?? "", /exported workflow publication publication-1/)
    const config = JSON.parse(await readFile(join(root, "exported", "publication.config.json"), "utf8"))
    assert.equal(config.publication_id, "publication-1")
    assert.equal(config.kernel_endpoint, "ws://kernel.example")
    assert.deepEqual(config.auth, { mode: "arroba", paired_senders: { enabled: true } })
    const launcher = await readFile(join(root, "exported", "run.sh"), "utf8")
    assert.match(launcher, /arroba-workflow-gateway/)
    const readme = await readFile(join(root, "exported", "README.md"), "utf8")
    assert.match(readme, /paired sender auth/)
    assert.match(readme, /well-known\/arroba\/publication\/pair/)
    assert.deepEqual(fake.requests, [
      { GetWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand manages advanced workflow settings, watchdogs, and queue", async () => {
  const workflow = makeWorkflow({ flush_agent_context_before_run: false, run_output_schema_ref: "final", intermediate_output_schema_ref: "progress" })
  const session = makeSession({ attachment_ids: ["attachment-1"], workflows: [workflow], workflow_launch_policy: "queue" })
  const node = { id: "node-1", agent_id: "agent-1", can_complete_workflow_run: true, max_turns: 3 }
  const watchdog = makeWorkflowWatchdog()
  const queued = { id: "queue-1", workflow_id: "workflow-1", endpoint_id: "endpoint-1", source: "manual" as const, queued_at_ms: 0 }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetSessionState" in request) {
          return { SessionState: { session } }
        }
        if ("SetWorkflowLaunchPolicy" in request) {
          return { WorkflowLaunchPolicyUpdated: { session } }
        }
        if ("SetWorkflowFlushContext" in request) {
          return { WorkflowFlushContextUpdated: { workflow, session } }
        }
        if ("SetWorkflowRunOutputSchema" in request) {
          return { WorkflowRunOutputSchemaUpdated: { workflow, session } }
        }
        if ("SetWorkflowNodeCanCompleteRun" in request) {
          return { WorkflowNodeCanCompleteRunUpdated: { node, workflow, session } }
        }
        if ("CreateWorkflowWatchdog" in request) {
          return { WorkflowWatchdogCreated: { watchdog, workflow, endpoint: workflow.endpoints![0], session } }
        }
        if ("ListWorkflowWatchdogs" in request) {
          return { WorkflowWatchdogsListed: { watchdogs: [watchdog] } }
        }
        if ("SetWorkflowWatchdogEnabled" in request) {
          return { WorkflowWatchdogUpdated: { watchdog: { ...watchdog, enabled: false }, session } }
        }
        if ("RemoveWorkflowWatchdog" in request) {
          return { WorkflowWatchdogRemoved: { watchdog, session } }
        }
        if ("ListQueuedWorkflowLaunches" in request) {
          return { QueuedWorkflowLaunchesListed: { queued_launches: [queued] } }
        }
        if ("RemoveQueuedWorkflowLaunch" in request) {
          return { QueuedWorkflowLaunchRemoved: { queued_launch: queued, session } }
        }
        if ("ClearQueuedWorkflowLaunches" in request) {
          return { QueuedWorkflowLaunchesCleared: { queued_launches: [queued], session } }
        }
        return { SessionConfigUpdated: { session, config: { version: 1, values: { "workflow.max_turns": "4" } } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", workflowId: "workflow-1" })
  const launchPolicy = await executeShellCommand(parseShellCommand("workflow launch-policy queue"), context, { client: fake.client })
  const flush = await executeShellCommand(parseShellCommand("workflow flush-context false"), context, { client: fake.client })
  const schema = await executeShellCommand(parseShellCommand("workflow run-output-schema final"), context, { client: fake.client })
  const maxTurns = await executeShellCommand(parseShellCommand("workflow max-turns 4"), context, { client: fake.client })
  const nodeConfig = await executeShellCommand(parseShellCommand("workflow node can-complete-run node-1 true"), context, { client: fake.client })
  const watchdogAdd = await executeShellCommand(parseShellCommand("workflow watchdog add endpoint-1 every 1m queue Run it"), context, { client: fake.client })
  const watchdogList = await executeShellCommand(parseShellCommand("workflow watchdog list workflow-1"), context, { client: fake.client })
  const watchdogDisable = await executeShellCommand(parseShellCommand("workflow watchdog disable watchdog-1"), context, { client: fake.client })
  const watchdogRemove = await executeShellCommand(parseShellCommand("workflow watchdog remove watchdog-1"), context, { client: fake.client })
  const queueList = await executeShellCommand(parseShellCommand("workflow queue list"), context, { client: fake.client })
  const queueRemove = await executeShellCommand(parseShellCommand("workflow queue remove queue-1"), context, { client: fake.client })
  const queueFlush = await executeShellCommand(parseShellCommand("workflow queue flush"), context, { client: fake.client })
  assert.equal(launchPolicy.ok, true)
  assert.match(launchPolicy.message ?? "", /launch policy set to queue/)
  assert.equal(flush.ok, true)
  assert.equal(schema.ok, true)
  assert.equal(maxTurns.ok, true)
  assert.equal(nodeConfig.ok, true)
  assert.equal(watchdogAdd.ok, true)
  assert.match(watchdogAdd.message ?? "", /created workflow watchdog watchdog-1/)
  assert.equal(watchdogList.ok, true)
  assert.match(watchdogList.message ?? "", /watchdog-1 workflow=workflow-1/)
  assert.equal(watchdogDisable.ok, true)
  assert.equal(watchdogRemove.ok, true)
  assert.equal(queueList.ok, true)
  assert.match(queueList.message ?? "", /queue-1 workflow=workflow-1/)
  assert.equal(queueRemove.ok, true)
  assert.equal(queueFlush.ok, true)
  assert.deepEqual(requests, [
    { SetWorkflowLaunchPolicy: { session_id: "session-1", policy: "queue" } },
    { SetWorkflowFlushContext: { session_id: "session-1", workflow_ref: "workflow-1", flush_agent_context_before_run: false } },
    { SetWorkflowRunOutputSchema: { session_id: "session-1", workflow_ref: "workflow-1", run_output_schema_ref: "final" } },
    { GetSessionState: { session_id: "session-1" } },
    { UpdateSessionConfig: { session_id: "session-1", attachment_id: "attachment-1", values: { "workflow.max_turns": "4" }, requires_idle: false } },
    { SetWorkflowNodeCanCompleteRun: { session_id: "session-1", workflow_ref: "workflow-1", node_id: "node-1", can_complete_workflow_run: true } },
    { CreateWorkflowWatchdog: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", interval_seconds: 60, invocation_prompt: "Run it", policy: "queue", max_wakeups_configured: false, max_wakeups: null } },
    { ListWorkflowWatchdogs: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { SetWorkflowWatchdogEnabled: { session_id: "session-1", watchdog_ref: "watchdog-1", enabled: false } },
    { RemoveWorkflowWatchdog: { session_id: "session-1", watchdog_ref: "watchdog-1" } },
    { ListQueuedWorkflowLaunches: { session_id: "session-1" } },
    { RemoveQueuedWorkflowLaunch: { session_id: "session-1", queue_item_ref: "queue-1" } },
    { ClearQueuedWorkflowLaunches: { session_id: "session-1" } },
  ])
})

test("executeShellCommand creates workflow watchdogs with explicit workflow ref", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [workflow] })
  const watchdog = makeWorkflowWatchdog()
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return { WorkflowWatchdogCreated: { watchdog, workflow, endpoint: workflow.endpoints![0], session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("workflow watchdog add workflow-1 endpoint-1 every 1m skip Run it"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { CreateWorkflowWatchdog: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", interval_seconds: 60, invocation_prompt: "Run it", policy: "skip", max_wakeups_configured: false, max_wakeups: null } },
  ])
})

test("executeShellCommand manages provider auth and processes", async () => {
  const process: ProviderProcessInfo = {
    process_id: "process-1",
    provider: "codex",
    process_label: "codex-agent",
    endpoint_mode: "managed",
    status: "idle",
    started_at_ms: 0,
    last_activity_at_ms: 0,
    provider_session_ids: [],
    owner_session_ids: ["session-1"],
    owner_provider_run_ids: [],
    attached_session_ids: [],
    active_workflow_run_ids: [],
    teardown_safe: true,
    teardown_blockers: [],
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("StartProviderLogin" in request) {
          return { ProviderLoginStarted: { login: { provider: "codex", login_kind: "device", login_id: "login-1", auth_url: null, verification_url: "https://auth.example", user_code: "ABCD" } } }
        }
        if ("LogoutProvider" in request) {
          return { ProviderLoggedOut: { provider: "codex" } }
        }
        if ("TeardownProviderProcesses" in request) {
          return { ProviderProcessesTornDown: { processes: [process] } }
        }
        return { ProviderProcessesListed: { processes: [process] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", provider: "codex" })
  const login = await executeShellCommand(parseShellCommand("provider login"), context, { client: fake.client })
  const logout = await executeShellCommand(parseShellCommand("provider logout codex"), context, { client: fake.client })
  const reauth = await executeShellCommand(parseShellCommand("provider reauth codex"), context, { client: fake.client })
  const list = await executeShellCommand(parseShellCommand("provider processes codex"), context, { client: fake.client })
  const teardown = await executeShellCommand(parseShellCommand("provider processes teardown codex"), context, { client: fake.client })
  assert.equal(login.ok, true)
  assert.match(login.message ?? "", /codex login started/)
  assert.equal(logout.ok, true)
  assert.equal(reauth.ok, true)
  assert.match(reauth.message ?? "", /codex reauth started/)
  assert.equal(list.ok, true)
  assert.match(list.message ?? "", /process-1 codex/)
  assert.equal(teardown.ok, true)
  assert.match(teardown.message ?? "", /tore down 1 provider process/)
  assert.deepEqual(requests, [
    { StartProviderLogin: { provider: "codex" } },
    { LogoutProvider: { provider: "codex" } },
    { LogoutProvider: { provider: "codex" } },
    { StartProviderLogin: { provider: "codex" } },
    { ListProviderProcesses: { provider: "codex" } },
    { TeardownProviderProcesses: { provider: "codex", force: false } },
  ])
})

test("executeShellCommand cancels active prompt through the current session attachment", async () => {
  const session = makeSession({ attachment_ids: ["attachment-1"] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetSessionState" in request) {
          return { SessionState: { session } }
        }
        return { PromptCancelled: { cancellation: { prompt: { id: "prompt-1" }, started_next: null } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("stop"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-1/)
  assert.deepEqual(requests, [
    { GetSessionState: { session_id: "session-1" } },
    { CancelActivePrompt: { session_id: "session-1", attachment_id: "attachment-1" } },
  ])
})

test("executeShellCommand cancels active prompt through shell context attachment", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return { PromptCancelled: { cancellation: { prompt: null, started_next: null } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", attachmentId: "attachment-shell" })
  const result = await executeShellCommand(parseShellCommand("stop"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { CancelActivePrompt: { session_id: "session-1", attachment_id: "attachment-shell" } },
  ])
})
