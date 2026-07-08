import {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
} from "../command-actions-workflow.test-support.js"
import type { RuntimeAttachment, RuntimeProviderRun, WorkflowDefinition, WorkflowQueuedPrompt, WorkflowRun } from "../command-actions-workflow.test-support.js"

test("root workflow shortcuts run registered templates", async () => {
  const calls: string[] = []
  const flashes: string[] = []
  const selectedWorkflowIds: (string | null)[] = []
  const screenOpens: string[] = []
  const session = makeSession()
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string, tone: string) => flashes.push(`${message}:${tone}`),
    selectWorkflowCanvas: (workflowId: string | null) => selectedWorkflowIds.push(workflowId),
    showWorkflowScreen: () => screenOpens.push("workflow"),
    runWorkflowRegistryEntry: async (
      name: string,
      endpointRef: string,
      prompt: string,
      queueRef: string | null,
      options?: { agentRebindings?: Array<{ node: string; agent_ref: string }> },
    ) => {
      const rebindings = options?.agentRebindings
        ?.map((rebinding) => `${rebinding.node}->${rebinding.agent_ref}`)
        .join(",") ?? "none"
      calls.push(`${name}:${endpointRef}:${prompt}:${queueRef ?? "default"}:${rebindings}`)
      return {
        entry: { name },
        result: {
          apply: {
            apply: {
              workflow_id: `workflow-${name}`,
              agent_ids: name === "loop-until-done"
                ? { worker: "agent-1", checker: "agent-checker" }
                : { planner: "agent-1", worker: "agent-worker", reviewer: "agent-reviewer" },
            },
          },
          invocation: { kind: "run_started" },
        },
        session,
      }
    },
  }))

  await handlers.handleLoopCommand({ kind: "loop", raw: "/loop Build a Kanban app", prompt: "Build a Kanban app" })
  await handlers.handleGoalCommand({ kind: "goal", raw: "/goal Build a Kanban app", prompt: "Build a Kanban app" })

  assert.deepEqual(calls, [
    "loop-until-done:entry:Build a Kanban app:default:worker->agent-1",
    "planner-worker-reviewer:entry:Build a Kanban app:default:planner->agent-1",
  ])
  assert.deepEqual(selectedWorkflowIds, ["workflow-loop-until-done", "workflow-planner-worker-reviewer"])
  assert.deepEqual(screenOpens, ["workflow", "workflow"])
  assert.deepEqual(flashes, [
    "ran workflow loop-until-done as workflow-loop-until-done; reused agent-1 as worker; spawned agent-checker [run_started]:info",
    "ran workflow planner-worker-reviewer as workflow-planner-worker-reviewer; reused agent-1 as planner; spawned agent-worker, agent-reviewer [run_started]:info",
  ])
})

test("root workflow shortcuts require a focused agent for entry-node reuse", async () => {
  const calls: string[] = []
  const flashes: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    focusedAgentId: () => null,
    flashFooter: (message: string, tone: string) => flashes.push(`${message}:${tone}`),
    runWorkflowRegistryEntry: async (name: string) => {
      calls.push(name)
      return {
        entry: { name },
        result: { apply: { apply: { workflow_id: "workflow-1" } }, invocation: { kind: "run_started" } },
        session: makeSession(),
      }
    },
  }))

  await handlers.handleLoopCommand({ kind: "loop", raw: "/loop Build a Kanban app", prompt: "Build a Kanban app" })
  await handlers.handleGoalCommand({ kind: "goal", raw: "/goal Build a Kanban app", prompt: "Build a Kanban app" })

  assert.deepEqual(calls, [])
  assert.deepEqual(flashes, [
    "/loop requires a focused agent in this session:error",
    "/goal requires a focused agent in this session:error",
  ])
})

test("root workflow shortcuts reject stale focused agent ids", async () => {
  const calls: string[] = []
  const flashes: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    focusedAgentId: () => "missing-agent",
    flashFooter: (message: string, tone: string) => flashes.push(`${message}:${tone}`),
    runWorkflowRegistryEntry: async (name: string) => {
      calls.push(name)
      return {
        entry: { name },
        result: { apply: { apply: { workflow_id: "workflow-1" } }, invocation: { kind: "run_started" } },
        session: makeSession(),
      }
    },
  }))

  await handlers.handleLoopCommand({ kind: "loop", raw: "/loop Build a Kanban app", prompt: "Build a Kanban app" })

  assert.deepEqual(calls, [])
  assert.deepEqual(flashes, ["/loop requires a focused agent in this session:error"])
})

test("root workflow shortcuts reject empty prompts", async () => {
  const calls: string[] = []
  const flashes: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string, tone: string) => flashes.push(`${message}:${tone}`),
    runWorkflowRegistryEntry: async (name: string) => {
      calls.push(name)
      return {
        entry: { name },
        result: { apply: { apply: { workflow_id: "workflow-1" } }, invocation: { kind: "run_started" } },
        session: makeSession(),
      }
    },
  }))

  await handlers.handleLoopCommand({ kind: "loop", raw: "/loop", prompt: "" })
  await handlers.handleGoalCommand({ kind: "goal", raw: "/goal", prompt: "" })

  assert.deepEqual(calls, [])
  assert.deepEqual(flashes, [
    "usage: /loop <prompt>:error",
    "usage: /goal <prompt>:error",
  ])
})

test("workflow add node all adds only agents missing from the selected workflow", async () => {
  const existingAgent = makeAgent({
    id: "agent-existing",
    agent_ref: "agent-existing",
  })
  const firstMissingAgent = makeAgent({
    id: "agent-missing-a",
    agent_ref: "agent-missing-a",
    alias: "reviewer",
  })
  const secondMissingAgent = makeAgent({
    id: "agent-missing-b",
    agent_ref: "agent-missing-b",
  })
  let workflow: WorkflowDefinition = {
    id: "workflow-1",
    alias: null,
    nodes: [{ id: "node-existing", agent_id: existingAgent.id }],
    edges: [],
    endpoints: [],
  }
  let flashedMessage = ""
  const addedAgentIds: string[] = []
  const selectedWorkflowIds: string[] = []
  const upsertedWorkflowNodeCounts: number[] = []
  const currentSession = () => makeSession({
    focused_agent_id: existingAgent.id,
    agents: [existingAgent, firstMissingAgent, secondMissingAgent],
    workflows: [workflow],
  })

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: currentSession,
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun | null => null,
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
    focusedAgentId: () => existingAgent.id,
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
    updateSessionResponseLayout: async () => ({ session: currentSession(), config: currentSession().config_state }),
    updateSessionConfig: async () => ({ session: currentSession(), config: currentSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: currentSession() }),
    launchAgentProviderRun: async () => { throw new Error("should not launch provider") },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession(),
    spawnAgent: async () => ({ agent: firstMissingAgent, session: currentSession() }),
    destroyAgent: async () => currentSession(),
    focusAgent: async () => ({ agent: existingAgent, session: currentSession() }),
    resolveSessionAgent: () => ({ agent: existingAgent }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectedWorkflowId: () => "workflow-1",
    selectWorkflowCanvas: (workflowId) => { selectedWorkflowIds.push(workflowId ?? "null") },
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: (nextWorkflow) => {
      workflow = nextWorkflow
      upsertedWorkflowNodeCounts.push(nextWorkflow.nodes?.length ?? 0)
    },
    createWorkflow: async () => ({ workflow, session: currentSession() }),
    listWorkflows: async () => [workflow],
    resolveWorkflow: async (workflowRef) => {
      if (workflowRef !== workflow.id) {
        throw new Error(`unknown workflow: ${workflowRef}`)
      }
      return { workflow }
    },
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({
      endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-existing" },
      workflow,
      session: currentSession(),
    }),
    assignWorkflowEndpointAlias: async () => ({
      endpoint: { id: "endpoint-1", alias: "entry", entry_node_id: "node-existing" },
      workflow,
      session: currentSession(),
    }),
    bindWorkflowEndpoint: async () => ({
      endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-existing" },
      workflow,
      session: currentSession(),
    }),
    addWorkflowNode: async (_workflowRef, agentId) => {
      addedAgentIds.push(agentId)
      const node = { id: `node-${addedAgentIds.length}`, agent_id: agentId }
      workflow = {
        ...workflow,
        nodes: [...(workflow.nodes ?? []), node],
      }
      return {
        node,
        workflow,
        session: currentSession(),
      }
    },
    removeWorkflowNode: async (_workflowRef, nodeId) => ({
      node: { id: nodeId, agent_id: existingAgent.id },
      workflow,
      session: currentSession(),
    }),
    addWorkflowEdge: async () => ({
      edge: { id: "edge-1", from_node_id: "node-existing", to_node_id: "node-1" },
      workflow,
      session: currentSession(),
    }),
    removeWorkflowEdge: async () => ({
      edge: { id: "edge-1", from_node_id: "node-existing", to_node_id: "node-1" },
      workflow,
      session: currentSession(),
    }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow add node all",
    args: ["add", "node", "all"],
  })

  assert.deepEqual(addedAgentIds, ["agent-missing-a", "agent-missing-b"])
  assert.deepEqual(workflow.nodes?.map((node) => node.agent_id), [
    "agent-existing",
    "agent-missing-a",
    "agent-missing-b",
  ])
  assert.deepEqual(selectedWorkflowIds, ["workflow-1"])
  assert.deepEqual(upsertedWorkflowNodeCounts, [1, 2, 3])
  assert.equal(flashedMessage, "added 2 workflow nodes for agent-missing-a, agent-missing-b")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow node add all",
    args: ["node", "add", "all"],
  })

  assert.deepEqual(addedAgentIds, ["agent-missing-a", "agent-missing-b"])
  assert.equal(flashedMessage, "workflow workflow-1 already has nodes for all session agents")
})

