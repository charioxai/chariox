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

test("executeShellCommand updates agent alias through dedicated alias request", async () => {
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const renamed = { ...agent, alias: "ui" }
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("AliasAgent" in request) {
      return { AgentAliased: { agent: renamed, session: makeSession({ agents: [renamed] }) } }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent alias reviewer ui"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(ui\) alias = ui/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      AliasAgent: {
        session_id: "session-1",
        agent_id: "agent-2",
        alias: "ui",
      },
    },
  ])
})

test("executeShellCommand updates agent provider profile through dedicated profile request", async () => {
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const updated = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "reviewer",
    provider: "codex",
    model: "gpt-5.4",
    effort: "low",
  })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentProfile" in request) {
      return { AgentProfileUpdated: { agent: updated, session: makeSession({ agents: [updated] }) } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent provider reviewer codex"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(reviewer\) provider = codex/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentProfile: {
        session_id: "session-1",
        agent_id: "agent-2",
        provider: "codex",
        model: null,
        effort: null,
        clear_effort: false,
      },
    },
  ])
})

test("executeShellCommand clears agent variant through dedicated profile request", async () => {
  const agent = makeAgent({ effort: "low" })
  const updated = makeAgent({ effort: null })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentProfile" in request) {
      return { AgentProfileUpdated: { agent: updated, session: makeSession({ agents: [updated] }) } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(parseShellCommand("agent variant none"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-1 variant = <none>/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentProfile: {
        session_id: "session-1",
        agent_id: "agent-1",
        provider: null,
        model: null,
        effort: null,
        clear_effort: true,
      },
    },
  ])
})

test("executeShellCommand updates agent mode through dedicated config request", async () => {
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const updated = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "reviewer",
    execution_mode_override: "plan",
  })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentConfig" in request) {
      return { AgentConfigUpdated: { agent: updated, session: makeSession({ agents: [updated] }) } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent mode reviewer plan"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(reviewer\) mode = plan \(agent\)/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentConfig: {
        session_id: "session-1",
        agent_id: "agent-2",
        execution_mode: "plan",
        clear_execution_mode: false,
        permission_level: null,
        clear_permission_level: false,
        workspace_id: null,
        clear_workspace_id: false,
        worktree_id: null,
        clear_worktree_id: false,
      },
    },
  ])
})

test("executeShellCommand clears agent mode override through dedicated config request", async () => {
  const agent = makeAgent({ execution_mode_override: "plan" })
  const updated = makeAgent({ execution_mode_override: null })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentConfig" in request) {
      return {
        AgentConfigUpdated: {
          agent: updated,
          session: makeSession({
            agents: [updated],
            config_state: { version: 1, values: { "agents.mode": "build" } },
          }),
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(parseShellCommand("agent mode inherit"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-1 mode = build \(session\)/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentConfig: {
        session_id: "session-1",
        agent_id: "agent-1",
        execution_mode: null,
        clear_execution_mode: true,
        permission_level: null,
        clear_permission_level: false,
        workspace_id: null,
        clear_workspace_id: false,
        worktree_id: null,
        clear_worktree_id: false,
      },
    },
  ])
})

test("executeShellCommand manages agent substitutes", async () => {
  const baseAgent = makeAgent()
  const substituteAgent = makeAgent({
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
    substitutes: [{ provider: "codex", model: "gpt-5.4", variant: "medium" }],
    active_substitute_index: 0,
  })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [baseAgent] } }
    }
    if ("UpdateAgentSubstitutes" in request) {
      const payload = request.UpdateAgentSubstitutes as {
        action: Record<string, unknown>
      }
      if ("Add" in payload.action) {
        return {
          AgentConfigUpdated: {
            agent: makeAgent({ substitutes: [{ provider: "codex", model: "gpt-5.4", variant: "medium" }] }),
            session: makeSession(),
          },
        }
      }
      if ("Activate" in payload.action) {
        return { AgentConfigUpdated: { agent: substituteAgent, session: makeSession({ agents: [substituteAgent] }) } }
      }
    }
    if ("LaunchProviderRun" in request) {
      return {
        ProviderRunLaunchAccepted: {
          provider_run: {
            id: "run-sub",
            session_id: "session-1",
            agent_instance_id: "agent-1",
            adapter_key: "codex",
            provider: "codex",
            account_profile: "default",
            model: "gpt-5.4",
            variant: "medium",
            usage_tokens_total: null,
            state: "Starting",
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const addResult = await executeShellCommand(
    parseShellCommand("agent substitute add codex gpt-5.4 --variant medium --kernel kernel-local --worktree /repo/sub"),
    context,
    { client: fake.client },
  )
  const activateResult = await executeShellCommand(
    parseShellCommand("agent substitute activate 0"),
    context,
    { client: fake.client },
  )
  assert.equal(addResult.ok, true)
  assert.match(addResult.message ?? "", /substitute added/)
  assert.equal(activateResult.ok, true)
  assert.match(activateResult.message ?? "", /activated substitute 0/)
  assert.deepEqual(fake.requests.map((request) => Object.keys(request)[0]), [
    "ListAgents",
    "UpdateAgentSubstitutes",
    "ListAgents",
    "UpdateAgentSubstitutes",
    "LaunchProviderRun",
  ])
  const addRequest = fake.requests[1]
  assert.ok(addRequest && "UpdateAgentSubstitutes" in addRequest)
  const addPayload = addRequest.UpdateAgentSubstitutes as { action: unknown }
  assert.deepEqual(addPayload.action, {
    Add: {
      provider: "codex",
      model: "gpt-5.4",
      variant: "medium",
      kernel_id: "kernel-local",
      worktree_id: "/repo/sub",
    },
  })
})

test("executeShellCommand spawns worker agent on kernel", async () => {
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
        execution_mode: null,
        permission_level: null,
        worktree_id: null,
        kernel_ref: "worker-1",
        slice_ref: null,
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
  const result = await executeShellCommand(parseShellCommand("agent spawn qa --kernel worker-1 as qa"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { qa: "agent-remote" })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-remote" })
})

test("executeShellCommand treats --slice off as local agent placement", async () => {
  const agent = makeAgent({
    id: "agent-local",
    agent_ref: "agent-local",
    alias: "local",
    worktree_id: "/repo",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, {
      SpawnAgent: {
        session_id: "session-1",
        provider: "codex",
        alias: "local",
        model: "gpt-5.2",
        effort: "low",
        execution_mode: null,
        permission_level: null,
        worktree_id: null,
        kernel_ref: null,
        slice_ref: null,
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
  const result = await executeShellCommand(parseShellCommand("agent spawn local gpt-5.2 --slice off"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, { agentId: "agent-local" })
})

test("executeShellCommand creates and starts a headed slice for agent spawn", async () => {
  const agent = makeAgent({
    id: "agent-slice",
    agent_ref: "agent-slice",
    alias: "qa",
    remote_execution: {
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return { SliceCreated: { slice: { id: "slice-1" } } }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-1" } } }
    }
    if ("SpawnAgent" in request) {
      return { AgentSpawned: { agent } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })
  const result = await executeShellCommand(
    parseShellCommand("agent spawn qa --slice new --slice-display headed --worktree ../repo-feature --branch feature/login"),
    context,
    {
      client: fake.client,
      prepareLocalGitWorktree: async () => "/repo-feature",
    },
  )

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "SpawnAgent"])
  const createRequest = requests[0] as { CreateSlice: { name: string } }
  assert.match(createRequest.CreateSlice.name, /^repo-feature-slice-/)
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
      display_mode: "headed",
      workspace_id: "/repo",
      worktree_id: "/repo-feature",
      workspace_mount: "/repo-feature",
      worker_kernel_ref: null,
      display_url: null,
      provider_auth: [],
    },
  })
  assert.deepEqual(requests[1], { StartSlice: { slice_ref: "slice-1" } })
  assert.deepEqual(requests[2], {
    SpawnAgent: {
      session_id: "session-1",
      provider: "codex",
      alias: "qa",
      model: "gpt-5.2",
      effort: "low",
      execution_mode: null,
      permission_level: null,
      worktree_id: "/repo-feature",
      kernel_ref: null,
      slice_ref: "slice-1",
      worktree_placement: null,
    },
  })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-slice" })
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
