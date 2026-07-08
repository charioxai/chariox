import {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
} from "../command-actions-workflow.test-support.js"
import type { RuntimeAttachment, RuntimeProviderRun, WorkflowDefinition, WorkflowQueuedPrompt, WorkflowRun } from "../command-actions-workflow.test-support.js"

test("workflow command opens the workflow screen and manages local workflows", async () => {
  let flashedMessage = ""
  let shownWorkflowScreen = 0
  let addedWorkflowNodeAgentId: string | null = null
  let addedWorkflowEdgeRefs: { fromNodeId: string; toNodeId: string } | null = null
  let addedWorkflowEdgeWorkflowRef: string | null = null
  let createdWorkflowEndpointArgs: { workflowRef: string; entryNodeId: string; alias: string | null | undefined } | null = null
  let createdWorkflowWatchdogArgs: {
    workflowRef: string
    endpointRef: string
    intervalSeconds: number
    invocationPrompt: string
    policy: "skip" | "queue"
    maxWakeups?: number | null | undefined
  } | null = null
  let invokedWorkflowRunArgs: { workflowRef: string; endpointRef: string; prompt: string | null | undefined } | null = null
  let workflowFlushContext = true
  let workflowRunOutputSchema: string | null = null
  let workflowNodeCanCompleteRun = false
  let workflowNodeMaxTurns: number | null = null
  let removedQueuedPromptRef: string | null = null
  let cancelledWorkflowRunRef: string | null = null
  let resumedWorkflowRunRef: string | null = null
  let openedWorkflowTerminalId: string | null = null
  const selectedWorkflowIds: string[] = []
  const workflows = new Map<string, WorkflowDefinition>()
  const workflowRuns: WorkflowRun[] = []
  const queuedWorkflowPrompts: WorkflowQueuedPrompt[] = [
    {
      id: "queued-1",
      queue_id: "default",
      workflow_id: "workflow-1",
      endpoint_id: "entry",
      prompt: "later prompt from endpoint invocation",
      source: "manual",
      status: "queued",
      created_at_ms: 1,
      updated_at_ms: 1,
    },
  ]
  const workflowPromptQueues = [{ id: "default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 1, updated_at_ms: 1 }]
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
  const plannerRef = resolvedWorkflowAgent.agent_ref
  const reviewerRef = reviewerAgent.agent_ref
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
    updateSessionConfig: async () => ({
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
    selectedWorkflowId: () => selectedWorkflowIds.at(-1) ?? null,
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
        flush_agent_context_before_run: workflowFlushContext,
        run_output_schema_ref: workflowRunOutputSchema,
        nodes: [
          {
            id: "node-1",
            agent_id: resolvedWorkflowAgent.id,
            can_complete_workflow_run: workflowNodeCanCompleteRun,
            max_turns: workflowNodeMaxTurns,
          },
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
    setWorkflowFlushContext: async (workflowRef, flushAgentContextBeforeRun) => {
      workflowFlushContext = flushAgentContextBeforeRun
      const workflow = {
        ...(workflows.get(workflowRef) ?? { id: workflowRef, alias: null }),
        flush_agent_context_before_run: workflowFlushContext,
      }
      workflows.set(workflowRef, workflow)
      return {
        workflow,
        session: makeSession({
          workflows: [...workflows.values()],
          
          workflow_queued_prompts: queuedWorkflowPrompts,
        }),
      }
    },
    setWorkflowRunOutputSchema: async (workflowRef, runOutputSchemaRef) => {
      workflowRunOutputSchema = runOutputSchemaRef
      const workflow = {
        ...(workflows.get(workflowRef) ?? { id: workflowRef, alias: null }),
        run_output_schema_ref: workflowRunOutputSchema,
      }
      workflows.set(workflowRef, workflow)
      return {
        workflow,
        session: makeSession({
          workflows: [...workflows.values()],
          
          workflow_queued_prompts: queuedWorkflowPrompts,
        }),
      }
    },
    createWorkflowEndpoint: async (workflowRef, entryNodeId, alias) => {
      createdWorkflowEndpointArgs = { workflowRef, entryNodeId, alias }
      return {
        endpoint: { id: "endpoint-1", alias: alias ?? null, entry_node_id: entryNodeId },
        workflow: workflows.get(workflowRef) ?? { id: workflowRef, alias: null },
        session: makeSession(),
      }
    },
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
        node: { id: "node-1", agent_id: agentId, can_complete_workflow_run: workflowNodeCanCompleteRun, max_turns: workflowNodeMaxTurns },
        workflow: { id: "workflow-1", alias: null },
        session: makeSession(),
      }
    },
    removeWorkflowNode: async (_workflowRef, nodeId) => ({
      node: { id: nodeId, agent_id: "agent-1", can_complete_workflow_run: workflowNodeCanCompleteRun, max_turns: workflowNodeMaxTurns },
      workflow: { id: "workflow-1", alias: null },
      session: makeSession(),
    }),
    setWorkflowNodeCanCompleteRun: async (workflowRef, nodeId, canCompleteWorkflowRun) => {
      workflowNodeCanCompleteRun = canCompleteWorkflowRun
      const workflow = workflows.get(workflowRef) ?? { id: workflowRef, alias: null, nodes: [] }
      const nodes = (workflow.nodes ?? []).map((node) =>
        node.id === nodeId ? { ...node, can_complete_workflow_run: canCompleteWorkflowRun } : node,
      )
      const nextWorkflow = { ...workflow, nodes }
      workflows.set(workflowRef, nextWorkflow)
      return {
        node: nodes.find((node) => node.id === nodeId) ?? { id: nodeId, agent_id: "agent-1", can_complete_workflow_run: canCompleteWorkflowRun },
        workflow: nextWorkflow,
        session: makeSession({ workflows: [...workflows.values()] }),
      }
    },
    setWorkflowNodeMaxTurns: async (workflowRef, nodeId, maxTurns) => {
      workflowNodeMaxTurns = maxTurns
      const workflow = workflows.get(workflowRef) ?? { id: workflowRef, alias: null, nodes: [] }
      const nodes = (workflow.nodes ?? []).map((node) =>
        node.id === nodeId ? { ...node, max_turns: maxTurns } : node,
      )
      const nextWorkflow = { ...workflow, nodes }
      workflows.set(workflowRef, nextWorkflow)
      return {
        node: nodes.find((node) => node.id === nodeId) ?? { id: nodeId, agent_id: "agent-1", max_turns: maxTurns },
        workflow: nextWorkflow,
        session: makeSession({ workflows: [...workflows.values()] }),
      }
    },
    addWorkflowEdge: async (_workflowRef, fromNodeId, toNodeId) => {
      addedWorkflowEdgeWorkflowRef = _workflowRef
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
    invokeWorkflowEndpoint: async (workflowRef, endpointRef, prompt) => {
      invokedWorkflowRunArgs = { workflowRef, endpointRef, prompt }
      const workflow_run: WorkflowRun = {
        id: "run-1",
        workflow_id: "workflow-1",
        endpoint_id: endpointRef,
        entry_node_id: "node-1",
        status: "Running",
        invocation_prompt: prompt ?? null,
        active_node_run_id: "node-run-1",
        node_runs: [
          {
            id: "node-run-1",
            node_id: "node-1",
            agent_id: resolvedWorkflowAgent.id,
            status: "Running",
            summary: null,
            created_at_ms: 0,
            started_at_ms: 0,
            completed_at_ms: null,
          },
        ],
        messages: [],
        created_at_ms: 0,
        started_at_ms: 0,
        completed_at_ms: null,
      }
      workflowRuns.splice(0, workflowRuns.length, workflow_run)
      return {
        workflow_run,
        workflow: workflows.get(workflowRef) ?? { id: workflowRef, alias: null },
        endpoint: { id: endpointRef, alias: null, entry_node_id: "node-1" },
        session: makeSession({ workflows: [...workflows.values()], workflow_runs: workflowRuns }),
      }
    },
    listWorkflowPromptQueues: async () => workflowPromptQueues,
    listQueuedWorkflowPrompts: async () => queuedWorkflowPrompts,
    removeQueuedWorkflowPrompt: async (queueItemRef: string) => {
      removedQueuedPromptRef = queueItemRef
      const index = queuedWorkflowPrompts.findIndex((item) => item.id === queueItemRef)
      const queued_prompt =
        index >= 0 ? queuedWorkflowPrompts.splice(index, 1)[0]! : queuedWorkflowPrompts[0]!
      return {
        queued_prompt,
        session: makeSession({
          
          workflow_queued_prompts: queuedWorkflowPrompts,
        }),
      }
    },
    clearWorkflowPromptQueue: async () => {
      const queued_prompts = queuedWorkflowPrompts.splice(0, queuedWorkflowPrompts.length)
      return {
        queued_prompts,
        session: makeSession({
          
          workflow_queued_prompts: queuedWorkflowPrompts,
        }),
      }
    },
    listWorkflowRuns: async () => workflowRuns,
    cancelWorkflowRun: async (workflowRunRef) => {
      cancelledWorkflowRunRef = workflowRunRef
      const workflow_run = {
        ...(workflowRuns.find((candidate) => candidate.id === workflowRunRef) ?? workflowRuns[0]!),
        id: workflowRunRef,
        status: "Stopped",
        active_node_run_id: null,
      }
      workflowRuns.splice(0, workflowRuns.length, workflow_run)
      return {
        workflow_run,
        session: makeSession({ workflows: [...workflows.values()], workflow_runs: workflowRuns }),
      }
    },
    resumeWorkflowRun: async (workflowRunRef) => {
      resumedWorkflowRunRef = workflowRunRef
      const workflow_run = {
        ...(workflowRuns.find((candidate) => candidate.id === workflowRunRef) ?? workflowRuns[0]!),
        id: workflowRunRef,
        status: "Running",
        active_node_run_id: "node-run-1",
      }
      workflowRuns.splice(0, workflowRuns.length, workflow_run)
      return {
        workflow_run,
        session: makeSession({ workflows: [...workflows.values()], workflow_runs: workflowRuns }),
      }
    },
    openWorkflowTerminalPanel: (workflowId) => {
      openedWorkflowTerminalId = workflowId
    },
    createWorkflowWatchdog: async (workflowRef, endpointRef, intervalSeconds, invocationPrompt, policy, maxWakeups) => {
      createdWorkflowWatchdogArgs = { workflowRef, endpointRef, intervalSeconds, invocationPrompt, policy, maxWakeups }
      return {
        watchdog: {
          id: "watchdog-1",
          workflow_id: workflowRef,
          endpoint_id: endpointRef,
          trigger: { kind: "interval", every_seconds: intervalSeconds },
          interval_seconds: intervalSeconds,
          invocation_prompt: invocationPrompt,
          overlap_policy: policy,
          policy,
          max_wakeups: maxWakeups ?? null,
          max_runs: maxWakeups ?? null,
          runs_started: 0,
          wakeups_executed: 0,
          enabled: true,
          next_run_at_ms: 1,
          pending_run: false,
          created_at_ms: 0,
          updated_at_ms: 0,
        },
        workflow: workflows.get(workflowRef) ?? { id: workflowRef, alias: null },
        endpoint: { id: endpointRef, alias: null, entry_node_id: "node-1" },
        session: makeSession({ workflows: [...workflows.values()] }),
      }
    },
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
    updateSessionConfig: async () => ({
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
    updateSessionConfig: async () => ({
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

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow show", args: ["show"] })
  assert.equal(flashedMessage, "workflow workflow-1 (review)")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow run entry summarize selected workflow",
    args: ["run", "entry", "summarize", "selected", "workflow"],
  })
  assert.deepEqual(invokedWorkflowRunArgs, {
    workflowRef: "workflow-1",
    endpointRef: "entry",
    prompt: "summarize selected workflow",
  })
  assert.equal(flashedMessage, "started workflow run run-1 [running]")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow run workflow-1 entry summarize changes",
    args: ["run", "workflow-1", "entry", "summarize", "changes"],
  })
  assert.deepEqual(invokedWorkflowRunArgs, {
    workflowRef: "workflow-1",
    endpointRef: "entry",
    prompt: "summarize changes",
  })
  assert.equal(flashedMessage, "started workflow run run-1 [running]")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow flush-context workflow-1",
    args: ["flush-context", "workflow-1"],
  })
  assert.equal(flashedMessage, "workflow workflow-1 flush-context: true")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow flush-context workflow-1 false",
    args: ["flush-context", "workflow-1", "false"],
  })
  assert.equal(flashedMessage, "workflow workflow-1 flush-context set to false")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow flush-context true",
    args: ["flush-context", "true"],
  })
  assert.equal(flashedMessage, "workflow workflow-1 flush-context set to true")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow run-output-schema workflow-1",
    args: ["run-output-schema", "workflow-1"],
  })
  assert.equal(flashedMessage, "workflow workflow-1 run-output-schema: none")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow run-output-schema workflow-1 /tmp/schema.json",
    args: ["run-output-schema", "workflow-1", "/tmp/schema.json"],
  })
  assert.equal(flashedMessage, "workflow workflow-1 run-output-schema set to /tmp/schema.json")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow run-output-schema /tmp/selected-schema.json",
    args: ["run-output-schema", "/tmp/selected-schema.json"],
  })
  assert.equal(flashedMessage, "workflow workflow-1 run-output-schema set to /tmp/selected-schema.json")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow queue",
    args: ["queue"],
  })
  assert.equal(
    flashedMessage,
    'workflow queues: default(default) priority=0 depth=1; prompts: queued-1 [manual] workflow=workflow-1 queue=default endpoint=entry status=queued prompt="later prompt from endpoint invocation"',
  )

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow queue remove queued-1",
    args: ["queue", "remove", "queued-1"],
  })
  assert.equal(removedQueuedPromptRef, "queued-1")
  assert.equal(flashedMessage, "removed queued workflow prompt queued-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow node can-complete-run workflow-1 node-1 true",
    args: ["node", "can-complete-run", "workflow-1", "node-1", "true"],
  })
  assert.equal(flashedMessage, "workflow node node-1 can-complete-run set to true")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow node max-turns workflow-1 node-1 2",
    args: ["node", "max-turns", "workflow-1", "node-1", "2"],
  })
  assert.equal(flashedMessage, "workflow node node-1 max-turns set to 2")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow node can-complete-run node-1 false",
    args: ["node", "can-complete-run", "node-1", "false"],
  })
  assert.equal(flashedMessage, "workflow node node-1 can-complete-run set to false")

  queuedWorkflowPrompts.push({
    id: "queued-2",
    queue_id: "default",
    workflow_id: "workflow-1",
    endpoint_id: "entry",
    prompt: "later",
    source: "manual",
    status: "queued",
    created_at_ms: 2,
    updated_at_ms: 2,
  })
  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow queue flush",
    args: ["queue", "flush"],
  })
  assert.equal(flashedMessage, "cleared 1 queued workflow prompt from default")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow runs workflow-1",
    args: ["runs", "workflow-1"],
  })
  assert.equal(flashedMessage, "workflow runs: run-1 [running]")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow cancel run-1",
    args: ["cancel", "run-1"],
  })
  assert.equal(cancelledWorkflowRunRef, "run-1")
  assert.equal(flashedMessage, "cancelled workflow run run-1 [stopped]")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow resume run-1",
    args: ["resume", "run-1"],
  })
  assert.equal(resumedWorkflowRunRef, "run-1")
  assert.equal(flashedMessage, "resumed workflow run run-1 [running]")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow terminal workflow-1",
    args: ["terminal", "workflow-1"],
  })
  assert.equal(openedWorkflowTerminalId, "workflow-1")
  assert.equal(flashedMessage, "opened workflow logs pane for workflow-1")

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow workflow-1 shipit", args: ["workflow-1", "shipit"] })
  assert.equal(flashedMessage, "workflow workflow-1 aliased as shipit")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow node add workflow-1 ${plannerRef}`,
    args: ["node", "add", "workflow-1", plannerRef],
  })
  assert.equal(flashedMessage, `added workflow node node-1 for agent ${plannerRef}`)
  assert.equal(addedWorkflowNodeAgentId, "agent-instance-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow node add ${plannerRef}`,
    args: ["node", "add", plannerRef],
  })
  assert.equal(flashedMessage, `added workflow node node-1 for agent ${plannerRef}`)
  assert.equal(addedWorkflowNodeAgentId, "agent-instance-1")

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow edge add workflow-1 node-1 node-2", args: ["edge", "add", "workflow-1", "node-1", "node-2"] })
  assert.equal(flashedMessage, "added workflow edge edge-1")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })
  assert.equal(addedWorkflowEdgeWorkflowRef, "workflow-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow edge add workflow-1 ${plannerRef} ${reviewerRef}`,
    args: ["edge", "add", "workflow-1", plannerRef, reviewerRef],
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
    raw: "/workflow edge add node-1 node-2",
    args: ["edge", "add", "node-1", "node-2"],
  })
  assert.equal(flashedMessage, "added workflow edge edge-1")
  assert.equal(addedWorkflowEdgeWorkflowRef, "workflow-1")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge remove workflow-1 edge-1",
    args: ["edge", "remove", "workflow-1", "edge-1"],
  })
  assert.equal(flashedMessage, "removed workflow edge edge-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow edge remove edge-1",
    args: ["edge", "remove", "edge-1"],
  })
  assert.equal(flashedMessage, "removed workflow edge edge-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow workflow-1 ${plannerRef} ${reviewerRef}`,
    args: ["workflow-1", plannerRef, reviewerRef],
  })
  assert.equal(flashedMessage, "added workflow edge edge-1")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow edge add workflow-1 ${plannerRef} ${reviewerRef}`,
    args: ["edge", "add", "workflow-1", plannerRef, reviewerRef],
  })
  assert.equal(flashedMessage, "workflow edge already exists between those nodes")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow edge add workflow-1 ${plannerRef} ${plannerRef}`,
    args: ["edge", "add", "workflow-1", plannerRef, plannerRef],
  })
  assert.equal(flashedMessage, "workflow edges must connect two different nodes")
  assert.deepEqual(addedWorkflowEdgeRefs, { fromNodeId: "node-1", toNodeId: "node-2" })

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow endpoint new node-1 selected-start", args: ["endpoint", "new", "node-1", "selected-start"] })
  assert.equal(flashedMessage, "created workflow endpoint endpoint-1")
  assert.deepEqual(createdWorkflowEndpointArgs, {
    workflowRef: "workflow-1",
    entryNodeId: "node-1",
    alias: "selected-start",
  })

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow endpoint new workflow-1 node-1 start", args: ["endpoint", "new", "workflow-1", "node-1", "start"] })
  assert.equal(flashedMessage, "created workflow endpoint endpoint-1")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow watchdog add entry every 5m queue max-wakeups 2 scheduled selected",
    args: ["watchdog", "add", "entry", "every", "5m", "queue", "max-wakeups", "2", "scheduled", "selected"],
  })
  assert.equal(flashedMessage, "created workflow watchdog watchdog-1")
  assert.deepEqual(createdWorkflowWatchdogArgs, {
    workflowRef: "workflow-1",
    endpointRef: "entry",
    intervalSeconds: 300,
    invocationPrompt: "scheduled selected",
    policy: "queue",
    maxWakeups: 2,
  })

  await handlers.handleWorkflowCommand({ kind: "workflow", raw: "/workflow missing shipit", args: ["missing", "shipit"] })
  assert.equal(flashedMessage, "unknown workflow: missing")
})
