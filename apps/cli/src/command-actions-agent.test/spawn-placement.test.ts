import {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
} from "../command-actions-agent.test-support.js"
import type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession } from "../command-actions-agent.test-support.js"

test("agent spawn count inherits session defaults for each spawn", async () => {
  const sourceAgent = makeAgent({
    id: "agent-source",
    agent_ref: "agent-source",
    provider: "opencode",
    model: "opencode/gpt-5.4",
  })
  let currentSession = makeSession({
    focused_agent_id: sourceAgent.id,
    agent_defaults: {
      provider: "codex",
      model: "codex/gpt-5.4",
      effort: "high",
      account_profile: "default",
      execution_mode: "build",
      permission_level: "yolo",
    },
    agents: [sourceAgent],
  })
  const spawnCalls: Array<{ provider: string | null | undefined; alias: string | undefined; model: string | null | undefined; effort: string | null | undefined }> = []
  const launchCalls: Array<{ provider: string; model: string; effort: string; agentId: string }> = []
  let refreshCount = 0
  let flashedMessage = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => currentSession,
    attachmentState: () => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => currentSession.focused_agent_id,
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    updateSessionConfig: async () => ({ session: currentSession, config: currentSession.config_state }),
    applySessionState: (session) => {
      currentSession = session
    },
    refreshAgentPanes: async () => {
      refreshCount += 1
    },
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
    launchAgentProviderRun: async (provider, model, effort, agentId) => {
      launchCalls.push({ provider, model, effort, agentId })
      return {
        id: `provider-run-${launchCalls.length}`,
        session_id: "session-1",
        agent_instance_id: agentId,
        adapter_key: provider,
        provider,
        account_profile: "default",
        model,
        variant: effort,
        usage_tokens_total: null,
        state: "running",
      }
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async (provider, alias, model, effort) => {
      spawnCalls.push({ provider, alias, model, effort })
      const resolvedProvider = provider ?? currentSession.agent_defaults?.provider ?? "opencode"
      const resolvedModel = model ?? currentSession.agent_defaults?.model ?? "test-model"
      const resolvedEffort = effort ?? currentSession.agent_defaults?.effort ?? "low"
      const agent = makeAgent({
        id: `agent-${spawnCalls.length}`,
        agent_ref: `agent-${spawnCalls.length}`,
        provider: resolvedProvider,
        model: resolvedModel,
        effort: resolvedEffort,
        state: "Focused",
      })
      currentSession = makeSession({
        agent_defaults: currentSession.agent_defaults!,
        focused_agent_id: agent.id,
        agents: [...currentSession.agents, agent],
      })
      return { agent, session: currentSession }
    },
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: currentSession.agents[0] ?? sourceAgent, session: currentSession }),
    resolveSessionAgent: () => ({ agent: currentSession.agents[0] ?? null }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn 2",
    args: ["spawn", "2"],
  })

  assert.deepEqual(spawnCalls, [
    { provider: undefined, alias: undefined, model: undefined, effort: undefined },
    { provider: undefined, alias: undefined, model: undefined, effort: undefined },
  ])
  assert.deepEqual(launchCalls, [
    { provider: "codex", model: "codex/gpt-5.4", effort: "high", agentId: "agent-1" },
    { provider: "codex", model: "codex/gpt-5.4", effort: "high", agentId: "agent-2" },
  ])
  assert.equal(refreshCount, 4)
  assert.equal(flashedMessage, "spawned 2 agents from session defaults")
  assert.equal(currentSession.focused_agent_id, "agent-2")
})

test("agent spawn passes local directory as worktree and launches locally", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined }> = []
  const launchCalls: Array<{ agentId: string }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string) => {
      spawnCalls.push({ worktreeId, machineRef })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        state: "Focused",
      })
      const session = makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] })
      return { agent, session }
    },
    launchAgentProviderRun: async (_provider: string, _model: string, _variant: string, agentId: string) => {
      launchCalls.push({ agentId })
      return {
        id: "provider-run-1",
        session_id: "session-1",
        agent_instance_id: agentId,
        adapter_key: "opencode",
        provider: "opencode",
        account_profile: "default",
        model: "openai/gpt-5",
        variant: "medium",
        usage_tokens_total: null,
        state: "running",
      }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --dir .",
    args: ["spawn", "review", "openai/gpt-5", "--dir", "."],
  })

  assert.equal(spawnCalls.length, 1)
  assert.equal(spawnCalls[0]?.worktreeId, process.cwd())
  assert.equal(spawnCalls[0]?.machineRef, undefined)
  assert.deepEqual(launchCalls, [{ agentId: "agent-2" }])
  assert.match(flashedMessage, /^spawned agent agent-2 \(review\) · local · worktree /)
})

test("agent spawn forwards local git worktree placement to the kernel", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined; placement: unknown }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string, placement?: unknown) => {
      spawnCalls.push({ worktreeId, machineRef, placement })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        state: "Focused",
      })
      return { agent, session: makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] }) }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --worktree ../feature --branch feature/test --from main",
    args: ["spawn", "review", "openai/gpt-5", "--worktree", "../feature", "--branch", "feature/test", "--from", "main"],
  })

  assert.deepEqual(spawnCalls, [{
    worktreeId: undefined,
    machineRef: undefined,
    placement: {
      target_directory: "../feature",
      branch: "feature/test",
      from_ref: "main",
    },
  }])
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · local")
})

test("agent spawn with machine requires directory and does not launch local provider", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined; placement: unknown }> = []
  const kernelChecks: string[] = []
  let launchCount = 0
  let providerRunStateSet: RuntimeProviderRun | null | "unset" = "unset"
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    listRemoteMachineKernels: async (machineRef: string) => {
      kernelChecks.push(machineRef)
      return [{
        kernel_id: "kernel-worker",
        machine_id: "machine-worker",
        accepting_remote_leases: true,
        available_providers: ["opencode"],
      }]
    },
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string, placement?: unknown) => {
      spawnCalls.push({ worktreeId, machineRef, placement })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
        state: "Focused",
      })
      const session = makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] })
      return { agent, session }
    },
    launchAgentProviderRun: async () => {
      launchCount += 1
      throw new Error("remote spawn should not launch local provider")
    },
    setProviderRunState: (run: RuntimeProviderRun | null) => { providerRunStateSet = run },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.deepEqual(kernelChecks, ["worker"])
  assert.deepEqual(spawnCalls, [{ worktreeId: "/srv/project", machineRef: "worker", placement: undefined }])
  assert.equal(launchCount, 0)
  assert.equal(providerRunStateSet, null)
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · remote machine-worker · worktree /srv/project")
})

test("agent spawn with machine can use the worker default directory", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string) => {
      spawnCalls.push({ worktreeId, machineRef })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
        state: "Focused",
      })
      return { agent, session: makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] }) }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review --machine worker",
    args: ["spawn", "review", "--machine", "worker"],
  })

  assert.deepEqual(spawnCalls, [{ worktreeId: undefined, machineRef: "worker" }])
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · remote machine-worker")
})

test("agent spawn rejects machine and reusable slice together before provisioning", async () => {
  let spawnCount = 0
  let createSliceCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    createSlice: async () => {
      createSliceCount += 1
      throw new Error("should not create slice")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --slice slice-existing --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--slice", "slice-existing", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(createSliceCount, 0)
  assert.equal(flashedMessage, "usage: /agent spawn uses either --machine/--kernel or a reusable --slice, not both")
})

test("agent spawn with machine blocks workers that reject remote leases", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: false,
      available_providers: ["opencode"],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no kernel accepting remote agents; next: enable remote leases on kernel kernel-worker or choose another worker")
})

test("agent spawn with machine names workers without provider CLIs", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: [],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no accepting kernel with provider CLIs; next: configure provider CLIs on kernel kernel-worker or choose another worker")
})

test("agent spawn with machine blocks workers without authenticated provider accounts", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: ["opencode"],
      provider_accounts: [{ provider: "opencode", state: "not_configured", alias: "daily" }],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no ready worker kernel with authenticated provider accounts; next: configure/import or refresh provider accounts on kernel kernel-worker or choose another worker")
})

test("agent spawn with machine prefers account recovery when selected provider is present but unauthenticated", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-codex",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: ["codex"],
    }, {
      kernel_id: "kernel-opencode",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: ["opencode"],
      provider_accounts: [{ provider: "opencode", state: "not_configured", alias: "daily" }],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no ready worker kernel with an authenticated opencode account; next: configure/import or refresh the opencode account on kernel kernel-opencode or choose another worker")
})

test("agent spawn with machine blocks unknown worker readiness", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      available_providers: ["opencode"],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no kernel with known remote readiness; next: run /machine kernels worker, refresh relay inventory, or choose another worker")
})

test("agent spawn with machine blocks workers without the selected provider", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: ["codex"],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no accepting kernel with provider opencode; next: choose a worker with opencode or change the agent provider")
})

test("agent spawn with machine forwards remote git worktree placement", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined; placement: unknown }> = []
  let launchCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (_provider: string, alias?: string, _model?: string, _effort?: string, worktreeId?: string, machineRef?: string, placement?: unknown) => {
      spawnCalls.push({ worktreeId, machineRef, placement })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        worktree_id: worktreeId ?? null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
      })
      return { agent, session: makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] }) }
    },
    launchAgentProviderRun: async () => {
      launchCount += 1
      throw new Error("remote spawn should not launch local provider")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --worktree /srv/project-feature --branch feature/remote --from main",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--worktree", "/srv/project-feature", "--branch", "feature/remote", "--from", "main"],
  })

  assert.deepEqual(spawnCalls, [{
    worktreeId: undefined,
    machineRef: "worker",
    placement: {
      target_directory: "/srv/project-feature",
      branch: "feature/remote",
      from_ref: "main",
    },
  }])
  assert.equal(launchCount, 0)
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · remote machine-worker")
})

