import assert from "node:assert/strict"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, ProviderProcessInfo, QueuedWorkflowLaunch, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"

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
    workflow_runs: [],
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

test("formatAgentCapabilityGrants renders MCP and skill grants", () => {
  const agent = makeAgent({
    alias: "qa",
    mcp_grants: ["browser", "github"],
    skill_grants: ["browser-qa"],
  })

  assert.equal(
    formatAgentCapabilityGrants(agent, "mcp"),
    "agent-1 (qa) MCP grants:\n- browser\n- github",
  )
  assert.equal(
    formatAgentCapabilityGrants(agent, "skill"),
    "agent-1 (qa) skill grants:\n- browser-qa",
  )
  assert.equal(
    formatAgentCapabilityGrants(makeAgent(), "skill"),
    "agent-1 has no skill grants.",
  )
})

test("parseMcpInstallConfig supports stdio and streamable HTTP MCPs", () => {
  assert.deepEqual(
    parseMcpInstallConfig(["install", "browser", "--command", "npx", "--arg", "-y", "--arg", "@modelcontextprotocol/server-browser", "--env", "BROWSER_TOKEN"]),
    {
      name: "browser",
      transport: {
        type: "stdio",
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-browser"],
        env: {},
        env_vars: ["BROWSER_TOKEN"],
      },
      enabled: true,
      required: false,
    },
  )
  assert.deepEqual(
    parseMcpInstallConfig(["install", "remote", "--url", "https://example.test/mcp", "--bearer-token-env-var", "REMOTE_MCP_TOKEN"]),
    {
      name: "remote",
      transport: {
        type: "streamable_http",
        url: "https://example.test/mcp",
        bearer_token_env_var: "REMOTE_MCP_TOKEN",
        http_headers: {},
        env_http_headers: {},
      },
      enabled: true,
      required: false,
    },
  )
  assert.equal(parseMcpInstallConfig(["install", "bad", "--command", "npx", "--url", "https://example.test/mcp"]), null)
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

test("provider command can switch backends and manage codex auth", async () => {
  const events: string[] = []
  let flashedMessage = ""
  let notice = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notice = message },
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyProviderSelection: async (value) => { events.push(`provider:${value}`) },
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    getProviderAuthStatus: async () => ({
      provider: "codex",
      auth_state: "authenticated",
      account_profile: "user@example.com",
      login_hint: null,
      detected_version: "codex-cli 0.118.0",
    }),
    startProviderLogin: async () => ({
      provider: "codex",
      login_kind: "chatgptDeviceCode",
      login_id: "login-1",
      auth_url: null,
      verification_url: "https://auth.openai.com/codex/device",
      user_code: "ABCD-1234",
    }),
    logoutProvider: async (provider) => ({ provider }),
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: makeSession(), config: makeSession().config_state }),
    updateSessionConfig: async () => ({ session: makeSession(), config: makeSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("unused") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
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
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider codex", value: "codex" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider status", value: "status" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider login", value: "login" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider logout", value: "logout" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider reauth", value: "reauth" })

  assert.deepEqual(events, ["provider:codex"])
  assert.equal(flashedMessage, "codex reauth started • code ABCD-1234 • https://auth.openai.com/codex/device")
  assert.equal(notice, "codex reauth started • code ABCD-1234 • https://auth.openai.com/codex/device")
})

test("provider processes command lists and tears down safe daemon-tracked processes", async () => {
  let flashedMessage = ""
  let notice = ""
  let listedProvider: string | null | undefined = undefined
  let tornDownProvider: string | null | undefined = undefined

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notice = message },
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyProviderSelection: async () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    getProviderAuthStatus: async () => ({
      provider: "codex",
      auth_state: "authenticated",
      account_profile: "user@example.com",
      login_hint: null,
      detected_version: "codex-cli 0.118.0",
    }),
    startProviderLogin: async () => ({
      provider: "codex",
      login_kind: "chatgptDeviceCode",
      login_id: "login-1",
      auth_url: null,
      verification_url: "https://auth.openai.com/codex/device",
      user_code: "ABCD-1234",
    }),
    logoutProvider: async (provider) => ({ provider }),
    listProviderProcesses: async (provider) => {
      listedProvider = provider
      return [
        {
          process_id: "codex:shared-token",
          provider: "codex",
          process_label: "codex:gpt-5.4",
          pid: 4321,
          endpoint_mode: "managed",
          status: "idle",
          started_at_ms: 1,
          last_activity_at_ms: 2,
          provider_session_ids: ["thread-1"],
          owner_session_ids: ["session-1"],
          owner_provider_run_ids: ["provider-run-1"],
          attached_session_ids: [],
          active_workflow_run_ids: [],
          teardown_safe: true,
          teardown_blockers: [],
        },
      ]
    },
    teardownProviderProcesses: async (provider) => {
      tornDownProvider = provider
      return [
        {
          process_id: "codex:shared-token",
          provider: "codex",
          process_label: "codex:gpt-5.4",
          pid: 4321,
          endpoint_mode: "managed",
          status: "idle",
          started_at_ms: 1,
          last_activity_at_ms: 2,
          provider_session_ids: ["thread-1"],
          owner_session_ids: ["session-1"],
          owner_provider_run_ids: ["provider-run-1"],
          attached_session_ids: [],
          active_workflow_run_ids: [],
          teardown_safe: true,
          teardown_blockers: [],
        },
      ]
    },
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: makeSession(), config: makeSession().config_state }),
    updateSessionConfig: async () => ({ session: makeSession(), config: makeSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("unused") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
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
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider processes codex", value: "processes codex" })
  assert.equal(listedProvider, "codex")
  assert.equal(flashedMessage, "listed 1 provider process(es)")
  assert.match(notice, /codex:shared-token provider=codex pid=4321 status=idle mode=managed safe=true/)
  assert.match(notice, /provider sessions: thread-1/)
  assert.match(notice, /owner runs: provider-run-1/)

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider processes teardown codex", value: "processes teardown codex" })
  assert.equal(tornDownProvider, "codex")
  assert.equal(flashedMessage, "tore down 1 provider process(es)")
  assert.match(notice, /codex:shared-token provider=codex pid=4321 status=idle mode=managed safe=true/)
})

test("provider processes teardown reports blocked daemon-tracked processes", async () => {
  let flashedMessage = ""
  let notices: string[] = []

  const blockedProcess: ProviderProcessInfo = {
    process_id: "codex:blocked",
    provider: "codex",
    process_label: "codex:gpt-5.4",
    pid: 5555,
    endpoint_mode: "managed",
    status: "active",
    started_at_ms: 1,
    last_activity_at_ms: 2,
    provider_session_ids: ["thread-blocked"],
    owner_session_ids: ["session-1"],
    owner_provider_run_ids: ["provider-run-blocked"],
    attached_session_ids: ["session-1"],
    active_workflow_run_ids: [],
    teardown_safe: false,
    teardown_blockers: ["attached sessions: session-1"],
  }

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notices.push(message) },
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyProviderSelection: async () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    getProviderAuthStatus: async () => ({ provider: "codex", auth_state: "authenticated", account_profile: null, login_hint: null, detected_version: null }),
    startProviderLogin: async () => ({ provider: "codex", login_kind: "chatgptDeviceCode", login_id: "login-1", auth_url: null, verification_url: null, user_code: null }),
    logoutProvider: async (provider) => ({ provider }),
    listProviderProcesses: async () => [blockedProcess],
    teardownProviderProcesses: async () => [],
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: makeSession(), config: makeSession().config_state }),
    updateSessionConfig: async () => ({ session: makeSession(), config: makeSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("unused") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
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
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider processes teardown codex", value: "processes teardown codex" })
  assert.equal(flashedMessage, "no safe provider processes to tear down")
  assert.match(notices[0]!, /blocked provider processes:/)
  assert.match(notices[0]!, /blockers: attached sessions: session-1/)
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
  assert.equal(flashedMessage, "spawned agent agent-2 (review)")
})

test("agent spawn count clones the originally focused agent provider and model for each spawn", async () => {
  const sourceAgent = makeAgent({
    id: "agent-source",
    agent_ref: "agent-source",
    provider: "opencode",
    model: "openai/gpt-5.4",
  })
  let currentSession = makeSession({
    focused_agent_id: sourceAgent.id,
    agents: [sourceAgent],
  })
  const spawnCalls: Array<{ provider: string; alias: string | undefined; model: string | undefined; effort: string | undefined }> = []
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
      const agent = makeAgent({
        id: `agent-${spawnCalls.length}`,
        agent_ref: `agent-${spawnCalls.length}`,
        provider,
        model: model ?? null,
        state: "Focused",
      })
      currentSession = makeSession({
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
    { provider: "opencode", alias: undefined, model: "openai/gpt-5.4", effort: "high" },
    { provider: "opencode", alias: undefined, model: "openai/gpt-5.4", effort: "high" },
  ])
  assert.deepEqual(launchCalls, [
    { provider: "opencode", model: "openai/gpt-5.4", effort: "high", agentId: "agent-1" },
    { provider: "opencode", model: "openai/gpt-5.4", effort: "high", agentId: "agent-2" },
  ])
  assert.equal(refreshCount, 4)
  assert.equal(flashedMessage, "spawned 2 agents from agent-source")
  assert.equal(currentSession.focused_agent_id, "agent-2")
})

  test("session command aliases the current session", async () => {
    let flashedMessage = ""
    let aliasedPayload: { sessionId: string; alias: string } | null = null
    let appliedSession: Pick<RuntimeSession, "alias"> | null = null
    const currentSession = makeSession()
    const handlers = createCommandActionHandlers({
      workspace: "workspace-1",
      worktree: "worktree-1",
      accountProfile: "default",
      isAttached: () => true,
      sessionState: () => currentSession,
      attachmentState: () => ({ id: "attachment-1", session_id: "session-1" }),
      providerRunState: () => null,
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
      updateSessionConfig: async () => ({ session: currentSession, config: currentSession.config_state }),
      assignSessionAlias: async (sessionId, alias) => {
        aliasedPayload = { sessionId, alias }
        return { ...currentSession, alias }
      },
      applySessionState: (session) => {
        appliedSession = session
      },
      refreshAgentPanes: async () => {},
      saveUiPreferences: async () => {},
      rebuildTranscript: () => {},
      requestRender: () => {},
      cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
      launchAgentProviderRun: async () => {
        throw new Error("unused")
      },
      setProviderRunState: () => {},
      refreshSessionState: async () => currentSession,
      spawnAgent: async () => ({ agent: makeAgent(), session: currentSession }),
      destroyAgent: async () => currentSession,
      focusAgent: async () => ({ agent: makeAgent(), session: currentSession }),
      resolveSessionAgent: () => ({ agent: null }),
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

  await handlers.handleSessionCommand({
    kind: "session",
    raw: "/session work-session",
    action: "work-session",
    args: [],
    value: "work-session",
  })

  assert.deepEqual(aliasedPayload, { sessionId: "session-1", alias: "work-session" })
  assert.equal(
    (appliedSession as (Pick<RuntimeSession, "alias"> | null))?.alias,
    "work-session",
  )
  assert.equal(flashedMessage, "session session-1 aliased as work-session")
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
  let workflowLaunchPolicy: "reject" | "queue" = "reject"
  let workflowFlushContext = true
  let workflowRunOutputSchema: string | null = null
  let workflowNodeCanCompleteRun = false
  let workflowNodeMaxTurns: number | null = null
  let removedQueuedLaunchRef: string | null = null
  let cancelledWorkflowRunRef: string | null = null
  let resumedWorkflowRunRef: string | null = null
  let openedWorkflowTerminalId: string | null = null
  const selectedWorkflowIds: string[] = []
  const workflows = new Map<string, WorkflowDefinition>()
  const workflowRuns: WorkflowRun[] = []
  const queuedWorkflowLaunches: QueuedWorkflowLaunch[] = [
    {
      id: "queued-1",
      workflow_id: "workflow-1",
      endpoint_id: "entry",
      invocation_prompt: "later prompt from endpoint invocation",
      source: "manual",
      queued_at_ms: 1,
    },
  ]
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
          workflow_launch_policy: workflowLaunchPolicy,
          queued_workflow_launches: queuedWorkflowLaunches,
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
          workflow_launch_policy: workflowLaunchPolicy,
          queued_workflow_launches: queuedWorkflowLaunches,
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
    setWorkflowLaunchPolicy: async (policy) => {
      workflowLaunchPolicy = policy
      return {
        session: makeSession({
          workflow_launch_policy: workflowLaunchPolicy,
          queued_workflow_launches: queuedWorkflowLaunches,
        }),
      }
    },
    listQueuedWorkflowLaunches: async () => queuedWorkflowLaunches,
    removeQueuedWorkflowLaunch: async (queueItemRef) => {
      removedQueuedLaunchRef = queueItemRef
      const index = queuedWorkflowLaunches.findIndex((item) => item.id === queueItemRef)
      const queued_launch =
        index >= 0 ? queuedWorkflowLaunches.splice(index, 1)[0]! : queuedWorkflowLaunches[0]!
      return {
        queued_launch,
        session: makeSession({
          workflow_launch_policy: workflowLaunchPolicy,
          queued_workflow_launches: queuedWorkflowLaunches,
        }),
      }
    },
    clearQueuedWorkflowLaunches: async () => {
      const queued_launches = queuedWorkflowLaunches.splice(0, queuedWorkflowLaunches.length)
      return {
        queued_launches,
        session: makeSession({
          workflow_launch_policy: workflowLaunchPolicy,
          queued_workflow_launches: queuedWorkflowLaunches,
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
          interval_seconds: intervalSeconds,
          invocation_prompt: invocationPrompt,
          policy,
          max_wakeups: maxWakeups ?? null,
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
    raw: "/workflow launch-policy",
    args: ["launch-policy"],
  })
  assert.equal(flashedMessage, "workflow launch policy: reject")

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
    raw: "/workflow launch-policy queue",
    args: ["launch-policy", "queue"],
  })
  assert.equal(flashedMessage, "workflow launch policy set to queue")

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow queue",
    args: ["queue"],
  })
  assert.equal(
    flashedMessage,
    'workflow queue: queued-1 [manual] workflow=workflow-1 endpoint=entry queued_at=1 prompt="later prompt from endpoint invocation"',
  )

  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow queue remove queued-1",
    args: ["queue", "remove", "queued-1"],
  })
  assert.equal(removedQueuedLaunchRef, "queued-1")
  assert.equal(flashedMessage, "removed queued workflow launch queued-1")

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

  queuedWorkflowLaunches.push({
    id: "queued-2",
    workflow_id: "workflow-1",
    endpoint_id: "entry",
    invocation_prompt: "later",
    source: "manual",
    queued_at_ms: 2,
  })
  await handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: "/workflow queue flush",
    args: ["queue", "flush"],
  })
  assert.equal(flashedMessage, "cleared 1 queued workflow launch")

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
  assert.equal(flashedMessage, "opened workflow terminal for workflow-1")

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
