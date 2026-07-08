import {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
} from "../command-actions-agent.test-support.js"
import type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession } from "../command-actions-agent.test-support.js"

test("agent mode updates the focused agent through shared agent config", async () => {
  const baseAgent = makeAgent()
  const updatedAgent = makeAgent({ execution_mode_override: "plan" })
  const updatedSession = makeSession({ agents: [updatedAgent] })
  const updateCalls: Array<{ sessionId: string; agentId: string; executionMode: string | null | undefined; clearExecutionMode: boolean | undefined }> = []
  const appliedSessions: RuntimeSession[] = []
  let refreshCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    resolveSessionAgent: () => ({ agent: baseAgent }),
    updateAgentConfig: async (sessionId: string, agentId: string, options: { executionMode?: "build" | "plan" | null; clearExecutionMode?: boolean }) => {
      updateCalls.push({
        sessionId,
        agentId,
        executionMode: options.executionMode,
        clearExecutionMode: options.clearExecutionMode,
      })
      return { agent: updatedAgent, session: updatedSession }
    },
    applySessionState: (session: RuntimeSession) => { appliedSessions.push(session) },
    refreshAgentPanes: async () => { refreshCount += 1 },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent mode plan",
    args: ["mode", "plan"],
  })

  assert.deepEqual(updateCalls, [{
    sessionId: "session-1",
    agentId: "agent-1",
    executionMode: "plan",
    clearExecutionMode: false,
  }])
  assert.deepEqual(appliedSessions, [updatedSession])
  assert.equal(refreshCount, 1)
  assert.equal(flashedMessage, "agent-1 mode: plan (agent)")
})

test("agent mode inherit clears the focused agent override", async () => {
  const overriddenAgent = makeAgent({ execution_mode_override: "plan" })
  const inheritedAgent = makeAgent({ execution_mode_override: null })
  const inheritedSession = makeSession({
    agents: [inheritedAgent],
    config_state: {
      version: 1,
      values: { "agents.mode": "build" },
      updated_by_attachment_id: "attachment-1",
    },
  })
  const updateCalls: Array<{ executionMode: string | null | undefined; clearExecutionMode: boolean | undefined }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    resolveSessionAgent: () => ({ agent: overriddenAgent }),
    updateAgentConfig: async (_sessionId: string, _agentId: string, options: { executionMode?: "build" | "plan" | null; clearExecutionMode?: boolean }) => {
      updateCalls.push({
        executionMode: options.executionMode,
        clearExecutionMode: options.clearExecutionMode,
      })
      return { agent: inheritedAgent, session: inheritedSession }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent mode inherit",
    args: ["mode", "inherit"],
  })

  assert.deepEqual(updateCalls, [{ executionMode: null, clearExecutionMode: true }])
  assert.equal(flashedMessage, "agent-1 mode: build (session)")
})

test("cycle agent focus keeps split pane contents stable within the same screen", async () => {
  const agentA = makeAgent({ id: "agent-a", agent_ref: "agent-a" })
  const agentB = makeAgent({ id: "agent-b", agent_ref: "agent-b" })
  const agentC = makeAgent({ id: "agent-c", agent_ref: "agent-c" })
  let currentSession = makeSession({
    focused_agent_id: "agent-a",
    agents: [agentA, agentB, agentC],
  })
  let refreshCount = 0
  let flashedMessage = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => currentSession,
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun | null => ({
      id: "provider-run-1",
      session_id: "session-1",
      agent_instance_id: "agent-a",
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
    },
    refreshAgentPanes: async () => {
      refreshCount += 1
    },
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({
      agent: agentB,
      session: {
        ...currentSession,
        active_provider_run_id: "provider-run-1",
        focused_agent_id: "agent-b",
      },
    }),
    launchAgentProviderRun: async () => {
      throw new Error("should not launch a new run")
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async () => ({ agent: agentB, session: currentSession }),
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: agentB, session: currentSession }),
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
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleCycleAgentFocus()

  assert.equal(refreshCount, 0)
  assert.equal(flashedMessage, "cycled to agent agent-b")
  assert.equal(currentSession.focused_agent_id, "agent-b")
})
