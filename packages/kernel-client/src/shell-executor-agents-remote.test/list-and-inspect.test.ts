import {
  assert,
  createDefaultShellContext,
  executeShellCommand,
  fakeClient,
  makeAgent,
  makeSession,
  parseShellCommand,
  test,
} from "../shell-executor-agents-remote.test-support.js"
import type { AgentInstance } from "../shell-executor-agents-remote.test-support.js"

test("executeShellCommand lists agents for current session", async () => {
  const agents = [makeAgent(), makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })]
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetSessionState: { session_id: "session-1" } })
    return { SessionState: { session: makeSession({
      agents,
      host_daemon_id: "home-kernel",
      host_machine_id: "home-machine",
      owner_user_id: "user-1",
      workspace_live_sync_mode: "tracked",
    }) } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /^session runtime: home kernel home-kernel@home-machine; owner user-1; authority home-owned; live sync tracked \(selected workspace\/worktree only; other repositories unrestricted\) on \/repo/)
  assert.match(result.message ?? "", /2 agents/)
  assert.match(result.message ?? "", /agent-1 \[Idle; opencode/)
  assert.match(result.message ?? "", /agent-2 \(reviewer\) \[Idle; opencode/)
  assert.match(result.message ?? "", /worktree \/repo; local; 0 grants/)
  assert.deepEqual((result.data as { agents: AgentInstance[] }).agents, agents)
})

test("executeShellCommand lists remote agents with slice placement and manifest state", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "worker",
    worktree_id: "/repo/feature",
    remote_execution: {
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-1",
    },
    extension_grants: [{ kind: "script", name: "deploy" }],
    remote_extension_manifest_sync: {
      state: "stale",
      manifest_hash: "abcdef1234567890",
      last_error: "worker lagging",
    },
  })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({
        agents: [agent],
        focused_agent_id: agent.id,
      }) } }
    }
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-wrong",
            name: "wrong-by-worker",
            owner_kernel_id: "kernel-local",
            owner_machine_id: "machine-local",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_mount: null,
            worker_kernel_ref: "slice:slice-wrong",
            worker_kernel_id: "slice-kernel",
            worker_machine_id: "slice-machine",
            agent_ids: ["agent-other"],
            created_at_ms: 0,
            updated_at_ms: 0,
          }, {
            id: "slice-1",
            name: "devbox",
            owner_kernel_id: "kernel-local",
            owner_machine_id: "machine-local",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_mount: null,
            worker_kernel_ref: "slice:slice-1",
            worker_kernel_id: "slice-kernel",
            worker_machine_id: "slice-machine",
            agent_ids: ["agent-remote"],
            provider_auth: [{
              provider: "opencode",
              state: "not_configured",
              alias: "backup",
              auth_type: "api",
              source: "slice",
            }],
            created_at_ms: 0,
            updated_at_ms: 0,
          }],
        },
      }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-remote \(worker\) \[Idle; opencode gpt-5\.2; worktree \/repo\/feature; slice devbox \(lease=lease-1, leased_agent=leased-agent-1, run=run-1\); auth opencode=backup \(api\)\/state=not_configured; refresh opencode; 1 grant \(active tools home-proxy\); manifest stale abcdef12 error worker lagging; see \/extension sync-status agent-remote\]/)
  assert.deepEqual(requests, [
    { GetSessionState: { session_id: "session-1" } },
    { ListSlices: null },
  ])
})

test("executeShellCommand lists blocked remote worker provider runs inline", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "worker",
    state: "Working",
    is_processing: true,
    worktree_id: "/repo/feature",
    remote_execution: {
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "hetzner",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: null,
    },
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({
        agents: [agent],
        focused_agent_id: agent.id,
        agent_activity: {
          [agent.id]: {
            status: "working",
            prompt_status: "running",
            busy: true,
            unread_idle_output: false,
            active_turn: {
              prompt_id: "prompt-remote",
              status: "running",
              phase: "streaming",
            },
          },
        },
        agent_activity_revision: 1,
      }) } }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [] } }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-remote \(worker\) \[Working; opencode gpt-5\.2; worktree \/repo\/feature; remote worker-kernel@hetzner \(lease=lease-1, leased_agent=leased-agent-1\); provider blocked \(missing worker run on hetzner; inspect agent-remote\); 0 grants\]/)
})

test("executeShellCommand lists remote skill-only agents without manifest pending", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "reviewer",
    remote_execution: {
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "worker-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    extension_grants: [{ kind: "skill", name: "review" }],
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({ agents: [agent], focused_agent_id: agent.id }) } }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [] } }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-remote \(reviewer\).*1 grant \(skills snapshot\)/)
  assert.doesNotMatch(result.message ?? "", /manifest pending/)
})

test("executeShellCommand inspects local agent placement and policy", async () => {
  const agent = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "reviewer",
    provider: "codex",
    model: "gpt-5.3",
    effort: "high",
    workspace_id: "/repo",
    worktree_id: "/repo-feature",
    execution_mode_override: "plan",
    permission_level_override: "required",
    extension_grants: [{ kind: "skill", name: "review" }],
  })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({
        agents: [agent],
        focused_agent_id: agent.id,
        active_provider_run_id: "run-session",
        host_daemon_id: "home-kernel",
        host_machine_id: "home-machine",
        owner_user_id: "user-1",
        workspace_live_sync_mode: "managed",
      }) } }
    }
    if ("GetProviderRun" in request) {
      return { ProviderRun: { provider_run: { id: "run-session", agent_instance_id: "agent-2" } } }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent inspect reviewer"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(reviewer\) \[Idle\]/)
  assert.match(result.message ?? "", /home kernel: home-kernel@home-machine/)
  assert.match(result.message ?? "", /session owner: user-1/)
  assert.match(result.message ?? "", /runtime authority: home owns session, prompts, grants, and live sync; workers execute leases and projected tools/)
  assert.match(result.message ?? "", /live sync: managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(result.message ?? "", /live sync scope: \/repo \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(result.message ?? "", /provider: codex/)
  assert.match(result.message ?? "", /worktree: \/repo-feature/)
  assert.match(result.message ?? "", /placement: worker-local/)
  assert.match(result.message ?? "", /provider run: session=run-session/)
  assert.match(result.message ?? "", /extensions: 1 grant \(worker-local; skill=1\)/)
  assert.match(result.message ?? "", /remote extension sync: not applicable \(worker-local agent; no home-proxy manifest\)/)
})

test("executeShellCommand inspects remote skill-only agents without manifest pending", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "reviewer",
    remote_execution: {
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "worker-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    extension_grants: [{ kind: "skill", name: "review" }],
  })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({ agents: [agent], focused_agent_id: agent.id }) } }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [] } }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent inspect reviewer"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /extensions: 1 grant \(skills snapshot; skill=1\)/)
  assert.match(result.message ?? "", /remote extension sync: not applicable \(no active home-proxy tools\)/)
  assert.doesNotMatch(result.message ?? "", /manifest pending/)
})

test("executeShellCommand reports unknown provider run owner when lookup fails", async () => {
  const agent = makeAgent({ id: "agent-1", agent_ref: "agent-1", alias: "focused" })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({ agents: [agent], focused_agent_id: agent.id, active_provider_run_id: "run-missing" }) } }
    }
    if ("GetProviderRun" in request) {
      throw new Error("provider run disappeared")
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent inspect focused"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /provider run: session=run-missing owner unknown; provider run disappeared/)
  assert.match(result.message ?? "", /provider run next: run \/kernel health and \/provider processes; export a debug bundle, then close or relaunch the mismatched provider run before sending more prompts to agent-1/)
})

test("executeShellCommand inspects remote agent lease and manifest state", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "slice qa",
    remote_execution: {
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-1",
    },
    extension_grants: [
      { kind: "mcp", name: "filesystem" },
      { kind: "script", name: "deploy" },
    ],
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "abcdef1234567890",
      last_error: "worker offline",
      pending_revoke: true,
    },
    substitutes: [{ provider: "opencode", model: "zen", variant: "fast" }],
    active_substitute_index: 0,
    last_substitution: {
      substitute_index: 0,
      reason: "Provider reported a substitutable resource limit: Insufficient balance",
      activated_at_ms: 1_700_000_000_000,
    },
  })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("GetSessionState" in request) {
      return { SessionState: { session: makeSession({ agents: [agent], focused_agent_id: agent.id, active_provider_run_id: "run-session", workspace_live_sync_mode: "tracked" }) } }
    }
    if ("GetProviderRun" in request) {
      return { ProviderRun: { provider_run: { id: "run-session", agent_instance_id: "agent-remote" } } }
    }
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-wrong",
            name: "wrong-by-worker",
            owner_kernel_id: "kernel-local",
            owner_machine_id: "machine-local",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headed",
            worktree_id: "/repo/other",
            workspace_mount: null,
            worker_kernel_ref: "slice:slice-wrong",
            worker_kernel_id: "slice-kernel",
            worker_machine_id: "slice-machine",
            agent_ids: ["agent-other"],
            created_at_ms: 0,
            updated_at_ms: 0,
          }, {
            id: "slice-1",
            name: "devbox",
            owner_kernel_id: "kernel-local",
            owner_machine_id: "machine-local",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headed",
            worktree_id: "/repo/feature",
            workspace_mount: null,
            worker_kernel_ref: "slice:slice-1",
            worker_kernel_id: "slice-kernel",
            worker_machine_id: "slice-machine",
            agent_ids: ["agent-remote", "agent-helper"],
            provider_auth: [{
              provider: "codex",
              state: "authenticated",
              email: "dev@example.com",
              alias: "daily",
            }],
            created_at_ms: 0,
            updated_at_ms: 0,
          }],
        },
      }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent inspect agent-remote"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /live sync: tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(result.message ?? "", /live sync scope: \/repo \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(result.message ?? "", /placement: slice devbox \(worker=slice-machine, kernel=slice-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=run-1\)/)
  assert.match(result.message ?? "", /provider run: session=run-session, worker=run-1/)
  assert.match(result.message ?? "", /slice: devbox \(id=slice-1, status=running, owner=kernel-local@machine-local, authority=home-managed, display=headed, worktree=\/repo\/feature, agents=2\)/)
  assert.match(result.message ?? "", /slice provider accounts: codex=daily \(dev@example.com\)/)
  assert.match(result.message ?? "", /extensions: 2 grants \(active tools home-proxy; mcp=1, script=1\)/)
  assert.match(result.message ?? "", /remote extension sync: failed, pending revoke, hash=abcdef123456, error=worker offline/)
  assert.match(result.message ?? "", /remote extension next: keep the home revoke in place; run \/extension sync-status agent-remote; run \/machine kernels slice-machine if the revoke stays pending; use \/extension sync-retry agent-remote after the worker reconnects/)
  assert.match(result.message ?? "", /substitutes: \*0:opencode\/zen\/fast/)
  assert.match(result.message ?? "", /last substitution: Provider reported a substitutable resource limit: Insufficient balance/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { GetSessionState: { session_id: "session-1" } },
    { ListSlices: null },
    { GetProviderRun: { provider_run_id: "run-session" } },
  ])
})
