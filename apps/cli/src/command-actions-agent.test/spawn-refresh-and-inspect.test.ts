import {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
} from "../command-actions-agent.test-support.js"
import type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession } from "../command-actions-agent.test-support.js"

test("agent spawn refreshes session state after launching the provider run", async () => {
  const firstAgent = makeAgent()
  const secondAgent = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "review",
    state: "Focused",
  })
  let currentSession = makeSession()
  const spawnedSession = makeSession({
    focused_agent_id: secondAgent.id,
    agents: [firstAgent, secondAgent],
  })
  const refreshedSession = {
    ...spawnedSession,
    active_provider_run_id: "provider-run-2",
  }
  const appliedProviderRunIds: Array<string | null> = []
  const refreshedPaneProviderRunIds: Array<string | null> = []
  let splitPaneRefreshCount = 0
  let flashedMessage = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => currentSession,
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun => ({
      id: "provider-run-1",
      session_id: "session-1",
      agent_instance_id: "agent-1",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "openai/gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "running",
    }),
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
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
    updateSessionConfig: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    applySessionState: (session) => {
      currentSession = session
      appliedProviderRunIds.push(session.active_provider_run_id)
    },
    refreshAgentPanes: async (session) => {
      refreshedPaneProviderRunIds.push(session.active_provider_run_id)
    },
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
    launchAgentProviderRun: async () => ({
      id: "provider-run-2",
      session_id: "session-1",
      agent_instance_id: "agent-2",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "openai/gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "running",
    }),
    setProviderRunState: () => {},
    refreshSessionState: async () => refreshedSession,
    spawnAgent: async () => ({ agent: secondAgent, session: spawnedSession }),
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: secondAgent, session: currentSession }),
    resolveSessionAgent: () => ({ agent: currentSession.agents[0] ?? null }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: "entry", entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => { splitPaneRefreshCount += 1 },
    formatSessionList: () => "",
  })

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review",
    args: ["spawn", "review"],
  })

  assert.deepEqual(appliedProviderRunIds, [null, "provider-run-2"])
  assert.deepEqual(refreshedPaneProviderRunIds, [null, "provider-run-2"])
  assert.equal(splitPaneRefreshCount, 1)
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · local · worktree worktree-1")
})

test("agent inspect renders diagnostics as a notice with concise footer", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "qa",
    remote_execution: {
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-1",
    },
    extension_grants: [{ kind: "mcp", name: "filesystem" }],
    remote_extension_manifest_sync: {
      state: "stale",
      manifest_hash: "abcdef123456",
      last_error: "worker offline",
    },
  })
  let flashedMessage = ""
  const notices: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => makeSession({ focused_agent_id: agent.id, agents: [agent] }),
    focusedAgentId: () => agent.id,
    resolveSessionAgent: () => ({ agent }),
    flashFooter: (message: string) => {
      flashedMessage = message
    },
    appendNotice: (message: string) => {
      notices.push(message)
    },
    listSlices: async () => [{
      id: "slice-1",
      name: "devbox",
      owner_kernel_id: "kernel-home",
      owner_machine_id: "machine-home",
      backend: "local_docker",
      os: "linux",
      status: "running",
      worktree_id: "worktree-1",
      worker_kernel_ref: "slice:slice-1",
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      agent_ids: ["agent-remote"],
      created_at_ms: 0,
      updated_at_ms: 0,
    }],
    formatAgentLabel: (entry: AgentInstance | null | undefined) => entry?.agent_ref ?? "",
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent inspect", args: ["inspect"] })

  assert.equal(flashedMessage, "showing agent agent-remote")
  assert.equal(notices.length, 1)
  assert.match(notices[0] ?? "", /placement: slice devbox \(worker=slice-machine, kernel=slice-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=run-1\)/)
  assert.match(notices[0] ?? "", /slice: devbox \(id=slice-1, status=running, owner=kernel-home@machine-home, authority=home-managed, display=headless, worktree=worktree-1, agents=1\)/)
  assert.match(notices[0] ?? "", /remote extension sync: stale, hash=abcdef123456, error=worker offline/)
})

test("agent inspect preserves slice lookup failures with recovery guidance", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    remote_execution: {
      worker_kernel_id: "slice:linux-dev",
      worker_machine_id: "hetzner",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const notices: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => makeSession({ focused_agent_id: agent.id, agents: [agent] }),
    focusedAgentId: () => agent.id,
    resolveSessionAgent: () => ({ agent }),
    appendNotice: (message: string) => {
      notices.push(message)
    },
    listSlices: async () => {
      throw new Error("kernel did not return slice inventory")
    },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
    formatAgentLabel: (entry: AgentInstance | null | undefined) => entry?.agent_ref ?? "",
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent inspect", args: ["inspect"] })

  assert.equal(notices.length, 1)
  assert.match(notices[0] ?? "", /placement: remote \(worker=hetzner, kernel=slice:linux-dev, lease=lease-1, leased_agent=leased-agent-1\)/)
  assert.match(notices[0] ?? "", /slice lookup: kernel did not return slice inventory/)
  assert.match(notices[0] ?? "", /slice next: run \/slice list; run \/slice doctor linux-dev if listed; run \/kernel remote-runtime if slice inventory stays unavailable/)
})

