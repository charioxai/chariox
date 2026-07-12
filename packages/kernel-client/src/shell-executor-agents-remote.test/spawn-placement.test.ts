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
        worktree_id: "/repo/qa",
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
  const result = await executeShellCommand(parseShellCommand("agent spawn qa --kernel worker-1 --dir qa as qa"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { qa: "agent-remote" })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-remote" })
})

test("executeShellCommand resolves machine spawn to a ready provider kernel", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "qa",
    remote_execution: {
      worker_kernel_id: "worker-ready",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListRemoteMachineKernels" in request) {
      return {
        RemoteMachineKernelsListed: {
          kernels: [{
            kernel_id: "worker-blocked",
            machine_id: "machine-1",
            accepting_remote_leases: false,
            available_providers: ["codex"],
          }, {
            kernel_id: "worker-ready",
            machine_id: "machine-1",
            accepting_remote_leases: true,
            available_providers: ["codex"],
            provider_accounts: [{ provider: "codex", state: "configured" }],
          }],
        },
      }
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

  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine machine-1"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests[0], { ListRemoteMachineKernels: { machine_ref: "machine-1" } })
  assert.equal((requests[1] as { SpawnAgent: { kernel_ref: string | null } }).SpawnAgent.kernel_ref, "worker-ready")
  assert.deepEqual(result.contextUpdates, { agentId: "agent-remote" })
})

test("executeShellCommand rejects machine spawn when no ready kernel supports the provider", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    return {
      RemoteMachineKernelsListed: {
        kernels: [{
          kernel_id: "worker-opencode",
          machine_id: "machine-1",
          accepting_remote_leases: true,
          available_providers: ["opencode"],
        }],
      },
    }
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })

  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine machine-1"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /no accepting kernel with provider codex/)
  assert.match(result.message ?? "", /next: run \/machine kernels machine-1; choose a ready worker with codex, configure\/import its provider account, or change the agent provider/)
  assert.deepEqual(requests, [{ ListRemoteMachineKernels: { machine_ref: "machine-1" } }])
})

test("executeShellCommand rejects machine spawn when provider account is not usable", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    return {
      RemoteMachineKernelsListed: {
        kernels: [{
          kernel_id: "worker-codex",
          machine_id: "machine-1",
          accepting_remote_leases: true,
          available_providers: ["codex"],
          provider_accounts: [{
            provider: "codex",
            state: "not_configured",
            auth_type: "api",
          }],
        }],
      },
    }
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })

  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine machine-1"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /no ready worker kernel with a usable codex account/)
  assert.match(result.message ?? "", /next: run \/machine kernels machine-1; configure\/import or refresh the codex account, or choose another worker/)
  assert.deepEqual(requests, [{ ListRemoteMachineKernels: { machine_ref: "machine-1" } }])
})

test("executeShellCommand forwards machine spawn directory placement after resolving kernels", async () => {
  const requests: Record<string, unknown>[] = []
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "qa",
    remote_execution: {
      worker_kernel_id: "worker-ready",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListRemoteMachineKernels" in request) {
      return {
        RemoteMachineKernelsListed: {
          kernels: [{
            kernel_id: "worker-ready",
            machine_id: "machine-1",
            accepting_remote_leases: true,
            available_providers: ["codex"],
          }],
        },
      }
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

  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine machine-1 --dir /repo"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests[0], { ListRemoteMachineKernels: { machine_ref: "machine-1" } })
  assert.equal((requests[1] as { SpawnAgent: { kernel_ref: string | null; worktree_id: string | null } }).SpawnAgent.kernel_ref, "worker-ready")
  assert.equal((requests[1] as { SpawnAgent: { kernel_ref: string | null; worktree_id: string | null } }).SpawnAgent.worktree_id, "/repo")
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

test("executeShellCommand creates a new slice on an explicit worker kernel", async () => {
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
    parseShellCommand("agent spawn qa --kernel worker-1 --slice new"),
    context,
    { client: fake.client },
  )

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "SpawnAgent"])
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
      worktree_id: "/repo",
      workspace_mount: "/repo",
      worker_kernel_ref: "worker-1",
      display_url: null,
      provider_auth: [],
      from_saved_state: null,
      base: null,
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
      worktree_id: null,
      kernel_ref: null,
      slice_ref: "slice-1",
      worktree_placement: null,
    },
  })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-slice" })
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
    },
  )

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "SpawnAgent"])
  const createRequest = requests[0] as { CreateSlice: { name: string } }
  assert.match(createRequest.CreateSlice.name, /^repo-slice-/)
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
      worktree_id: "/repo",
      workspace_mount: "/repo",
      worker_kernel_ref: null,
      display_url: null,
      provider_auth: [],
      from_saved_state: null,
      base: null,
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
      worktree_id: null,
      kernel_ref: null,
      slice_ref: "slice-1",
      worktree_placement: {
        target_directory: "../repo-feature",
        branch: "feature/login",
        from_ref: null,
      },
    },
  })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-slice" })
})
