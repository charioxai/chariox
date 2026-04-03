import assert from "node:assert/strict"
import test from "node:test"

import { createCommandActionHandlers, formatAgentListSummary, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition } from "./cli-types.js"

function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "openai/gpt-5",
    worktree_id: "worktree-1",
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
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: ["attachment-1"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [makeAgent()],
    workflows: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

test("parseRequestedViewLayout handles summary, invalid, and set cases", () => {
  assert.deepEqual(parseRequestedViewLayout("", "split"), { kind: "summary" })
  assert.deepEqual(parseRequestedViewLayout("grid", "split"), { kind: "invalid" })
  assert.deepEqual(parseRequestedViewLayout("individual", "split"), {
    kind: "set",
    layout: "individual",
  })
})

test("formatAgentListSummary renders aliases and pluralization", () => {
  const agents: AgentInstance[] = [
    {
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: "session-1",
      alias: "planner",
      provider: "opencode",
      model: "openai/gpt-5",
      worktree_id: "worktree-1",
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 0,
      last_activity_at_ms: 0,
    },
  ]

  assert.equal(formatAgentListSummary([]), "no agents in session")
  assert.equal(
    formatAgentListSummary(agents),
    "1 agent: agent-1 (planner) [Idle]",
  )
})

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
  assert.equal(flashedMessage, "spawned agent agent-2 (review)")
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

test("workflow command opens the workflow screen and manages local workflows", async () => {
  let flashedMessage = ""
  let shownWorkflowScreen = 0
  let addedWorkflowNodeAgentId: string | null = null
  let addedWorkflowEdgeRefs: { fromNodeId: string; toNodeId: string } | null = null
  const selectedWorkflowIds: string[] = []
  const workflows = new Map<string, WorkflowDefinition>()
  const resolvedWorkflowAgent = makeAgent({
    id: "agent-instance-1",
    agent_ref: "5f26c340",
    alias: "planner",
  })
  const reviewerAgent = makeAgent({
    id: "agent-instance-2",
    agent_ref: "19c82a89",
    alias: "reviewer",
  })
  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun | null => null,
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
    focusedAgentId: () => "agent-1",
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
      session: makeSession(),
      config: makeSession().config_state,
    }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("should not launch provider") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: (reference) => {
      if (
        reference === resolvedWorkflowAgent.id
        || reference === resolvedWorkflowAgent.agent_ref
        || reference === resolvedWorkflowAgent.alias
      ) {
        return { agent: resolvedWorkflowAgent }
      }
      if (
        reference === reviewerAgent.id
        || reference === reviewerAgent.agent_ref
        || reference === reviewerAgent.alias
      ) {
        return { agent: reviewerAgent }
      }
      return { agent: null, error: `agent '${reference ?? ""}' not found` }
    },
    workflowScreenActive: () => false,
    showWorkflowScreen: () => { shownWorkflowScreen += 1 },
    selectWorkflowCanvas: (workflowId) => { selectedWorkflowIds.push(workflowId ?? "null") },
    replaceWorkflowDefinitions: (nextWorkflows) => {
      workflows.clear()
      for (const workflow of nextWorkflows) {
        workflows.set(workflow.id, workflow)
      }
    },
    upsertWorkflowDefinition: (workflow) => {
      workflows.set(workflow.id, workflow)
    },
    createWorkflow: async (alias) => {
      const workflow = {
        id: "workflow-1",
        alias: alias ?? null,
        nodes: [
          { id: "node-1", agent_id: resolvedWorkflowAgent.id },
          { id: "node-2", agent_id: reviewerAgent.id },
        ],
        edges: [],
        endpoints: [],
      }
      const session = makeSession({ workflows: [workflow] })
      workflows.set(workflow.id, workflow)
      return { workflow, session }
    },
    listWorkflows: async () => [...workflows.values()],
    resolveWorkflow: async (workflowRef) => {
      const workflow = [...workflows.values()].find((item) => item.id === workflowRef || item.alias === workflowRef)
      if (!workflow) {
        throw new Error(`unknown workflow: ${workflowRef}`)
      }
      return { workflow }
    },
    assignWorkflowAlias: async (workflowId, alias) => {
      const workflow = workflows.get(workflowId)
      if (!workflow) {
        return null
      }
      const next = { ...workflow, alias }
      workflows.set(workflowId, next)
      return next
    },
    createWorkflowEndpoint: async (workflowRef, entryNodeId, alias) => ({
      endpoint: { id: "endpoint-1", alias: alias ?? null, entry_node_id: entryNodeId },
      workflow: workflows.get(workflowRef) ?? { id: workflowRef, alias: null },
      session: makeSession(),
    }),
    assignWorkflowEndpointAlias: async (_workflowRef, endpointRef, alias) => ({
      endpoint: { id: endpointRef, alias, entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    bindWorkflowEndpoint: async (_workflowRef, endpointRef, entryNodeId) => ({
      endpoint: { id: endpointRef, alias: null, entry_node_id: entryNodeId },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    addWorkflowNode: async (_workflowRef, agentId) => {
      addedWorkflowNodeAgentId = agentId
      return {
        node: { id: "node-1", agent_id: agentId },
        workflow: { id: "workflow-1", alias: null },
        session: makeSession(),
      }
    },
    removeWorkflowNode: async (_workflowRef, nodeId) => ({
      node: { id: nodeId, agent_id: "agent-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    addWorkflowEdge: async (_workflowRef, fromNodeId, toNodeId) => {
      addedWorkflowEdgeRefs = { fromNodeId, toNodeId }
      const edge = { id: "edge-1", from_node_id: fromNodeId, to_node_id: toNodeId }
      const currentWorkflow = workflows.get(_workflowRef) ?? { id: _workflowRef, alias: null }
      workflows.set(_workflowRef, {
        ...currentWorkflow,
        edges: [...(currentWorkflow.edges ?? []), edge],
      })
      return {
        edge,
        workflow: { id: "workflow-1", alias: null },
        session: makeSession(),
      }
    },
    removeWorkflowEdge: async (_workflowRef, edgeId) => ({
      edge: (() => {
        const currentWorkflow = workflows.get(_workflowRef) ?? { id: _workflowRef, alias: null }
        const existingEdges = currentWorkflow.edges ?? []
        const found = existingEdges.find((edge) => edge.id === edgeId) ?? {
          id: edgeId,
          from_node_id: "node-1",
          to_node_id: "node-2",
        }
        workflows.set(_workflowRef, {
          ...currentWorkflow,
          edges: existingEdges.filter((edge) => edge.id !== edgeId),
        })
        return found
      })(),
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow", args: [] })
  assert.equal(shownWorkflowScreen, 1)

  // Test: when workflow screen is already active and no workflows exist, create a workflow
  let createdWorkflowFromEmpty = false
  let activeScreenFlashedMessage = ""
  const activeScreenSelectedWorkflowIds: string[] = []
  const activeScreenWorkflows = new Map<string, WorkflowDefinition>()
  const handlersWithActiveScreen = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun | null => null,
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { activeScreenFlashedMessage = message },
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
      session: makeSession(),
      config: makeSession().config_state,
    }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("should not launch provider") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
    workflowScreenActive: () => true,  // Screen is already active
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: (workflowId: string | null) => { activeScreenSelectedWorkflowIds.push(workflowId ?? "null") },
    replaceWorkflowDefinitions: (nextWorkflows) => {
      activeScreenWorkflows.clear()
      for (const workflow of nextWorkflows) {
        activeScreenWorkflows.set(workflow.id, workflow)
      }
    },
    upsertWorkflowDefinition: (workflow) => {
      activeScreenWorkflows.set(workflow.id, workflow)
    },
    createWorkflow: async (alias: string | null | undefined) => {
      createdWorkflowFromEmpty = true
      const workflow = { id: "workflow-empty", alias: alias ?? null }
      activeScreenWorkflows.set(workflow.id, workflow)
      return { workflow, session: makeSession({ workflows: [workflow] }) }
    },
    listWorkflows: async () => [],  // No workflows exist
    resolveWorkflow: async (workflowRef: string) => {
      const workflow = [...activeScreenWorkflows.values()].find((item) => item.id === workflowRef || item.alias === workflowRef)
      if (!workflow) {
        throw new Error(`unknown workflow: ${workflowRef}`)
      }
      return { workflow }
    },
    assignWorkflowAlias: async (workflowId: string, alias: string) => {
      const workflow = activeScreenWorkflows.get(workflowId)
      if (!workflow) {
        return null
      }
      const next = { ...workflow, alias }
      activeScreenWorkflows.set(workflowId, next)
      return next
    },
    createWorkflowEndpoint: async () => ({
      endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    assignWorkflowEndpointAlias: async () => ({
      endpoint: { id: "endpoint-1", alias: "test", entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    bindWorkflowEndpoint: async () => ({
      endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    addWorkflowNode: async () => ({
      node: { id: "node-1", agent_id: "agent-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    removeWorkflowNode: async () => ({
      node: { id: "node-1", agent_id: "agent-1" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    addWorkflowEdge: async () => ({
      edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    removeWorkflowEdge: async () => ({
      edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })
  await handlersWithActiveScreen.handleWorkflowCommand({ kind: "workflow", raw: "/workflow", args: [] })
  assert.equal(createdWorkflowFromEmpty, true, "should create workflow when screen active but no workflows exist")
  assert.equal(activeScreenFlashedMessage, "created workflow workflow-empty")
  assert.deepEqual(activeScreenSelectedWorkflowIds, ["workflow-empty"])

  let hydratedWorkflows: WorkflowDefinition[] = []
  const hydratedSelections: string[] = []
  const handlersWithDetachedWorkflowCache = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun | null => null,
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: () => {},
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
      session: makeSession(),
      config: makeSession().config_state,
    }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("should not launch provider") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
    workflowScreenActive: () => true,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: (workflowId: string | null) => { hydratedSelections.push(workflowId ?? "null") },
    replaceWorkflowDefinitions: (workflows) => {
      hydratedWorkflows = workflows
    },
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => {
      throw new Error("should not create a workflow when the workspace already has one")
    },
    listWorkflows: async () => [{ id: "workflow-cached", alias: "cached", nodes: [], edges: [], endpoints: [] }],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-cached", alias: "cached", nodes: [], edges: [], endpoints: [] } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({
      endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    assignWorkflowEndpointAlias: async () => ({
      endpoint: { id: "endpoint-1", alias: "test", entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    bindWorkflowEndpoint: async () => ({
      endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    addWorkflowNode: async () => ({
      node: { id: "node-1", agent_id: "agent-1" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    removeWorkflowNode: async () => ({
      node: { id: "node-1", agent_id: "agent-1" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    addWorkflowEdge: async () => ({
      edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    removeWorkflowEdge: async () => ({
      edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" },
      workflow: { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      session: makeSession(),
    }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })
  await handlersWithDetachedWorkflowCache.handleWorkflowCommand({ kind: "workflow", raw: "/workflow", args: [] })
  assert.deepEqual(hydratedWorkflows.map((workflow) => workflow.id), ["workflow-cached"])
  assert.deepEqual(hydratedSelections, ["workflow-cached"])

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow new review", args: ["new", "review"] })
  assert.equal(flashedMessage, "created workflow workflow-1 (review)")
  assert.deepEqual(selectedWorkflowIds, ["workflow-1"])

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow workflow-1 shipit", args: ["workflow-1", "shipit"] })
  assert.equal(flashedMessage, "workflow workflow-1 aliased as shipit")

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow node add workflow-1 5f26c340", args: ["node", "add", "workflow-1", "5f26c340"] })
  assert.equal(flashedMessage, "added workflow node node-1 for agent 5f26c340")
  assert.equal(addedWorkflowNodeAgentId, "agent-instance-1")

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow edge add workflow-1 node-1 node-2", args: ["edge", "add", "workflow-1", "node-1", "node-2"] })
  assert.equal(flashedMessage, "added workflow edge edge-1")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge add workflow-1 5f26c340 19c82a89",
    args: ["edge", "add", "workflow-1", "5f26c340", "19c82a89"],
  })
  assert.equal(flashedMessage, "workflow edge already exists between those nodes")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge remove workflow-1 edge-1",
    args: ["edge", "remove", "workflow-1", "edge-1"],
  })
  assert.equal(flashedMessage, "removed workflow edge edge-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow workflow-1 5f26c340 19c82a89",
    args: ["workflow-1", "5f26c340", "19c82a89"],
  })
  assert.equal(flashedMessage, "added workflow edge edge-1")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge add workflow-1 5f26c340 19c82a89",
    args: ["edge", "add", "workflow-1", "5f26c340", "19c82a89"],
  })
  assert.equal(flashedMessage, "workflow edge already exists between those nodes")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge add workflow-1 5f26c340 5f26c340",
    args: ["edge", "add", "workflow-1", "5f26c340", "5f26c340"],
  })
  assert.equal(flashedMessage, "workflow edges must connect two different nodes")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge add workflow-1 5f26c340 19c82a89",
    args: ["edge", "add", "workflow-1", "5f26c340", "19c82a89"],
  })
  assert.equal(flashedMessage, "workflow edge already exists between those nodes")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow endpoint new workflow-1 node-1 start", args: ["endpoint", "new", "workflow-1", "node-1", "start"] })
  assert.equal(flashedMessage, "created workflow endpoint endpoint-1")

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow missing shipit", args: ["missing", "shipit"] })
  assert.equal(flashedMessage, "unknown workflow: missing")
})
