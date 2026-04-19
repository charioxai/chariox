import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowRun,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"

function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.2",
    worktree_id: "/repo",
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
    workspace_id: "/repo",
    worktree_id: "/repo",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [makeAgent()],
    config_state: { version: 0, values: {} },
    ...overrides,
  }
}

function makeWorkflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: "qa",
    flush_agent_context_before_run: true,
    nodes: [{ id: "node-1", agent_id: "agent-1" }],
    edges: [],
    endpoints: [{ id: "endpoint-1", alias: "default", entry_node_id: "node-1" }],
    ...overrides,
  }
}

function makeWorkflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status: "Running",
    invocation_prompt: "Run QA",
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 0,
    started_at_ms: 0,
    completed_at_ms: null,
    ...overrides,
  }
}

function fakeClient(handler: (request: Record<string, unknown>) => Record<string, unknown>) {
  const requests: Record<string, unknown>[] = []
  return {
    requests,
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return handler(request)
      },
    },
  }
}

test("executeShellCommand handles shell-local context mutations", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("set model gpt-5.3"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, { model: "gpt-5.3" })
  const next = applyShellCommandResult(context, result)
  assert.equal(next.model, "gpt-5.3")
})

test("executeShellCommand removes shell-local variables", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    variables: { stale: "agent-1", keep: "session-1" },
  })
  const result = await executeShellCommand(parseShellCommand("unset stale"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.variableRemovals, ["stale"])
  const next = applyShellCommandResult(context, result)
  assert.deepEqual(next.variables, { keep: "session-1" })
})

test("executeShellCommand creates a session and binds assignment", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { s: "session-2" })
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: "agent-1",
    workspace: "/repo",
    worktree: "/repo/qa",
  })
})

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

test("executeShellCommand spawns remote agent with worktree placement", async () => {
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
        worktree_id: "/remote/qa",
        machine_ref: "mac-mini",
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
  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine mac-mini --dir /remote/qa as qa"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { qa: "agent-remote" })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-remote" })
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

test("executeShellCommand reports relay status", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { RelayStatus: null })
    return {
      RelayStatus: {
        status: {
          configured: true,
          connected: false,
          relay_url: "wss://relay.example",
          relay_token_configured: true,
          daemon_id: "daemon-1",
          machine_id: "machine-1",
          machine_alias: "mini",
        },
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("relay status"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /configured, disconnected/)
  assert.match(result.message ?? "", /machine=mini/)
})

test("executeShellCommand lists MCP servers and skills in the workspace", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListMcpServers" in request) {
          return { McpServersListed: { mcps: [{ name: "playwright", transport: { stdio: { command: "npx" } }, enabled: true }] } }
        }
        return { SkillsListed: { skills: [{ name: "qa", description: "QA checks", path: "/skills/qa" }] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const mcpResult = await executeShellCommand(parseShellCommand("mcp list"), context, { client: fake.client })
  const skillResult = await executeShellCommand(parseShellCommand("skill list"), context, { client: fake.client })
  assert.equal(mcpResult.ok, true)
  assert.match(mcpResult.message ?? "", /playwright \[enabled\]/)
  assert.equal(skillResult.ok, true)
  assert.match(skillResult.message ?? "", /qa - QA checks/)
  assert.deepEqual(requests, [
    { ListMcpServers: { workspace_id: "/repo" } },
    { ListSkills: { workspace_id: "/repo" } },
  ])
})

test("executeShellCommand shows config and provider auth status", async () => {
  const fake = fakeClient((request) => {
    if ("GetUserConfig" in request) {
      return { UserConfig: { path: "/home/.arroba/config.json", config: { version: 1, providers: { default: "codex" } } } }
    }
    assert.deepEqual(request, { GetProviderAuthStatus: { provider: "codex" } })
    return {
      ProviderAuthStatus: {
        status: {
          provider: "codex",
          auth_state: "authenticated",
          account_profile: "default",
          login_hint: null,
          detected_version: "1.2.3",
        },
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", provider: "codex" })
  const configResult = await executeShellCommand(parseShellCommand("config show"), context, { client: fake.client })
  const providerResult = await executeShellCommand(parseShellCommand("provider status"), context, { client: fake.client })
  assert.equal(configResult.ok, true)
  assert.match(configResult.message ?? "", /"default": "codex"/)
  assert.equal(providerResult.ok, true)
  assert.match(providerResult.message ?? "", /codex: authenticated as default/)
  assert.match(providerResult.message ?? "", /version 1.2.3/)
})

test("executeShellCommand mutates user config", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetUserConfig" in request) {
          return { UserConfig: { path: "/home/.arroba/config.json", config: { version: 1, providers: { default: "codex" } } } }
        }
        return {
          UserConfigUpdated: {
            path: "/home/.arroba/config.json",
            config: { version: 1, providers: { managed_io: { codex: "required" } } },
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const pathResult = await executeShellCommand(parseShellCommand("config path"), context, { client: fake.client })
  const setResult = await executeShellCommand(parseShellCommand("config set providers.default opencode"), context, { client: fake.client })
  const unsetResult = await executeShellCommand(parseShellCommand("config unset providers.default"), context, { client: fake.client })
  const managedIoResult = await executeShellCommand(parseShellCommand("config managed-io codex on"), context, { client: fake.client })
  assert.equal(pathResult.ok, true)
  assert.equal(pathResult.message, "/home/.arroba/config.json")
  assert.equal(setResult.ok, true)
  assert.match(setResult.message ?? "", /config providers.default set to opencode/)
  assert.equal(unsetResult.ok, true)
  assert.match(unsetResult.message ?? "", /config providers.default unset/)
  assert.equal(managedIoResult.ok, true)
  assert.match(managedIoResult.message ?? "", /managed I\/O for codex set to required/)
  assert.deepEqual(requests, [
    { GetUserConfig: null },
    { SetUserConfigValue: { path: "providers.default", value: "opencode" } },
    { UnsetUserConfigValue: { path: "providers.default" } },
    { SetUserConfigValue: { path: "providers.managed_io.codex", value: "required" } },
  ])
})

test("executeShellCommand installs and updates MCP servers", async () => {
  const installed: ArrobaMcpServerConfig = {
    name: "playwright",
    transport: { type: "stdio", command: "npx", args: ["@playwright/mcp"], env: {}, env_vars: ["GITHUB_TOKEN"] },
    enabled: true,
    required: false,
  }
  const updated: ArrobaMcpServerConfig = {
    name: "browser",
    transport: {
      type: "streamable_http",
      url: "https://mcp.example",
      bearer_token_env_var: "MCP_TOKEN",
      http_headers: {},
      env_http_headers: {},
    },
    enabled: true,
    required: false,
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InstallMcpServer" in request) {
          return { McpServerInstalled: { mcp: installed } }
        }
        return { McpServerUpdated: { mcp: updated } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const installResult = await executeShellCommand(
    parseShellCommand("mcp install playwright --command npx --arg @playwright/mcp --env GITHUB_TOKEN"),
    context,
    { client: fake.client },
  )
  const updateResult = await executeShellCommand(
    parseShellCommand("mcp update browser --url https://mcp.example --bearer-token-env-var MCP_TOKEN"),
    context,
    { client: fake.client },
  )
  assert.equal(installResult.ok, true)
  assert.match(installResult.message ?? "", /installed MCP playwright/)
  assert.equal(updateResult.ok, true)
  assert.match(updateResult.message ?? "", /updated MCP browser/)
  assert.deepEqual(requests, [
    {
      InstallMcpServer: {
        workspace_id: "/repo",
        config: installed,
      },
    },
    {
      UpdateMcpServer: {
        workspace_id: "/repo",
        config: updated,
      },
    },
  ])
})

test("executeShellCommand imports MCP servers and skills", async () => {
  const mcp: ArrobaMcpServerConfig = {
    name: "github",
    transport: { type: "stdio", command: "github-mcp-server", args: [], env: {}, env_vars: [] },
    enabled: true,
    required: false,
  }
  const skill: ArrobaSkillMetadata = {
    name: "qa",
    description: "QA checks",
    short_description: "QA",
    path: "/skills/qa",
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ImportMcpServers" in request) {
          return {
            McpServersImported: {
              outcome: {
                imported: [mcp],
                skipped: [{ name: "oauth", reason: "oauth transport is provider-native" }],
              },
            },
          }
        }
        return {
          SkillsImported: {
            outcome: {
              imported: [skill],
              skipped: [{ name: "old", reason: "already installed" }],
            },
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const mcpResult = await executeShellCommand(parseShellCommand("mcp import codex github"), context, { client: fake.client })
  const skillResult = await executeShellCommand(parseShellCommand("skill import codex qa"), context, { client: fake.client })
  assert.equal(mcpResult.ok, true)
  assert.match(mcpResult.message ?? "", /Imported MCPs: github/)
  assert.match(mcpResult.message ?? "", /oauth: oauth transport is provider-native/)
  assert.equal(skillResult.ok, true)
  assert.match(skillResult.message ?? "", /Imported skills: qa/)
  assert.match(skillResult.message ?? "", /old: already installed/)
  assert.deepEqual(requests, [
    { ImportMcpServers: { workspace_id: "/repo", provider: "codex", name: "github" } },
    { ImportSkills: { workspace_id: "/repo", provider: "codex", name: "qa" } },
  ])
})

test("executeShellCommand grants, revokes, and lists agent capabilities", async () => {
  const agent = makeAgent({ mcp_grants: ["playwright"], skill_grants: ["qa"] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GrantAgentCapability" in request) {
          return { AgentCapabilityGranted: { agent } }
        }
        if ("RevokeAgentCapability" in request) {
          return { AgentCapabilityRevoked: { agent } }
        }
        return { AgentsListed: { agents: [agent] } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const grantResult = await executeShellCommand(parseShellCommand("mcp grant agent-1 playwright"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("skill revoke agent-1 qa"), context, { client: fake.client })
  const grantsResult = await executeShellCommand(parseShellCommand("mcp grants"), context, { client: fake.client })
  assert.equal(grantResult.ok, true)
  assert.match(grantResult.message ?? "", /granted MCP playwright to agent-1/)
  assert.deepEqual(grantResult.contextUpdates, { agentId: "agent-1" })
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked skill qa from agent-1/)
  assert.equal(grantsResult.ok, true)
  assert.match(grantsResult.message ?? "", /agent-1 MCP grants/)
  assert.match(grantsResult.message ?? "", /playwright/)
  assert.deepEqual(requests, [
    { GrantAgentCapability: { workspace_id: "/repo", agent_ref: "agent-1", kind: "mcp", name: "playwright" } },
    { RevokeAgentCapability: { agent_ref: "agent-1", kind: "skill", name: "qa" } },
    { ListAgents: { session_id: "session-1" } },
  ])
})

test("executeShellCommand installs and uninstalls skills", async () => {
  const skill: ArrobaSkillMetadata = {
    name: "qa",
    description: "QA checks",
    short_description: "QA",
    path: "/skills/qa",
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InstallSkill" in request) {
          return { SkillInstalled: { skill } }
        }
        return { SkillUninstalled: { skill } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const installResult = await executeShellCommand(parseShellCommand("skill install /tmp/skills/qa"), context, { client: fake.client })
  const uninstallResult = await executeShellCommand(parseShellCommand("skill uninstall qa"), context, { client: fake.client })
  assert.equal(installResult.ok, true)
  assert.match(installResult.message ?? "", /installed skill qa/)
  assert.equal(uninstallResult.ok, true)
  assert.match(uninstallResult.message ?? "", /uninstalled skill qa/)
  assert.deepEqual(requests, [
    { InstallSkill: { workspace_id: "/repo", source_path: "/tmp/skills/qa" } },
    { UninstallSkill: { workspace_id: "/repo", name: "qa" } },
  ])
})

test("executeShellCommand manages workflow list, create, show, and alias", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [workflow] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListWorkflows" in request) {
          return { WorkflowsListed: { workflows: [workflow] } }
        }
        if ("CreateWorkflow" in request) {
          return { WorkflowCreated: { workflow, session } }
        }
        if ("ResolveWorkflow" in request) {
          return { WorkflowResolved: { workflow } }
        }
        return { WorkflowAliased: { workflow: { ...workflow, alias: "review" }, session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const listResult = await executeShellCommand(parseShellCommand("workflow list"), context, { client: fake.client })
  const newResult = await executeShellCommand(parseShellCommand("workflow new qa as wf"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workflow show workflow-1"), context, { client: fake.client })
  const aliasResult = await executeShellCommand(parseShellCommand("workflow alias workflow-1 review"), context, { client: fake.client })
  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /workflow-1 \(qa\) nodes=1/)
  assert.equal(newResult.ok, true)
  assert.deepEqual(newResult.bindings, { wf: "workflow-1" })
  assert.deepEqual(newResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
  assert.equal(showResult.ok, true)
  assert.match(showResult.message ?? "", /workflow workflow-1 \(qa\)/)
  assert.deepEqual(showResult.contextUpdates, { workflowId: "workflow-1" })
  assert.equal(aliasResult.ok, true)
  assert.match(aliasResult.message ?? "", /aliased as review/)
  assert.deepEqual(requests, [
    { ListWorkflows: { session_id: "session-1" } },
    { CreateWorkflow: { session_id: "session-1", alias: "qa" } },
    { ResolveWorkflow: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { AliasWorkflow: { session_id: "session-1", workflow_ref: "workflow-1", alias: "review" } },
  ])
})

test("executeShellCommand runs and controls workflow runs", async () => {
  const workflow = makeWorkflow()
  const workflowRun = makeWorkflowRun()
  const session = makeSession({ workflows: [workflow], workflow_runs: [workflowRun] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InvokeWorkflowEndpoint" in request) {
          return { WorkflowRunInvoked: { workflow_run: workflowRun, workflow, endpoint: workflow.endpoints![0], session } }
        }
        if ("ListWorkflowRuns" in request) {
          return { WorkflowRunsListed: { workflow_runs: [workflowRun] } }
        }
        if ("GetWorkflowRun" in request) {
          return { WorkflowRun: { workflow_run: workflowRun } }
        }
        if ("CancelWorkflowRun" in request) {
          return { WorkflowRunCancelled: { workflow_run: { ...workflowRun, status: "Cancelled" }, session } }
        }
        return { WorkflowRunResumed: { workflow_run: { ...workflowRun, status: "Running" }, session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const runResult = await executeShellCommand(parseShellCommand("workflow run workflow-1 endpoint-1 Run QA"), context, { client: fake.client })
  const runsResult = await executeShellCommand(parseShellCommand("workflow runs workflow-1"), context, { client: fake.client })
  const showRunResult = await executeShellCommand(parseShellCommand("workflow run-show run-1"), context, { client: fake.client })
  const cancelResult = await executeShellCommand(parseShellCommand("workflow cancel run-1"), context, { client: fake.client })
  const resumeResult = await executeShellCommand(parseShellCommand("workflow resume run-1"), context, { client: fake.client })
  assert.equal(runResult.ok, true)
  assert.match(runResult.message ?? "", /started workflow run run-1/)
  assert.deepEqual(runResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
  assert.equal(runsResult.ok, true)
  assert.match(runsResult.message ?? "", /run-1 workflow=workflow-1/)
  assert.equal(showRunResult.ok, true)
  assert.equal(showRunResult.format, "json")
  assert.equal(cancelResult.ok, true)
  assert.match(cancelResult.message ?? "", /cancelled workflow run run-1 \[cancelled\]/)
  assert.equal(resumeResult.ok, true)
  assert.match(resumeResult.message ?? "", /resumed workflow run run-1 \[running\]/)
  assert.deepEqual(requests, [
    { InvokeWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", prompt: "Run QA" } },
    { ListWorkflowRuns: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { GetWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
    { CancelWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
    { ResumeWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
  ])
})

test("executeShellCommand manages workflow graph and endpoints", async () => {
  const workflow = makeWorkflow({
    nodes: [
      { id: "node-1", agent_id: "agent-1" },
      { id: "node-2", agent_id: "agent-2" },
    ],
    edges: [{ id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }],
  })
  const session = makeSession({ workflows: [workflow] })
  const node = { id: "node-2", agent_id: "agent-2" }
  const edge = { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }
  const endpoint = { id: "endpoint-1", alias: "default", entry_node_id: "node-1" }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListAgents" in request) {
          return { AgentsListed: { agents: [makeAgent(), makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })] } }
        }
        if ("AddWorkflowNode" in request) {
          return { WorkflowNodeAdded: { node, workflow, session } }
        }
        if ("RemoveWorkflowNode" in request) {
          return { WorkflowNodeRemoved: { node, workflow, session } }
        }
        if ("AddWorkflowEdge" in request) {
          return { WorkflowEdgeAdded: { edge, workflow, session } }
        }
        if ("RemoveWorkflowEdge" in request) {
          return { WorkflowEdgeRemoved: { edge, workflow, session } }
        }
        if ("CreateWorkflowEndpoint" in request) {
          return { WorkflowEndpointCreated: { endpoint, workflow, session } }
        }
        if ("AliasWorkflowEndpoint" in request) {
          return { WorkflowEndpointAliased: { endpoint: { ...endpoint, alias: "smoke" }, workflow, session } }
        }
        return { WorkflowEndpointBound: { endpoint, workflow, session } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    workflowId: "workflow-1",
  })
  const nodeAdd = await executeShellCommand(parseShellCommand("workflow node add reviewer as node"), context, { client: fake.client })
  const nodeRemove = await executeShellCommand(parseShellCommand("workflow node remove node-2"), context, { client: fake.client })
  const edgeAdd = await executeShellCommand(parseShellCommand("workflow edge add node-1 node-2"), context, { client: fake.client })
  const edgeRemove = await executeShellCommand(parseShellCommand("workflow edge remove edge-1"), context, { client: fake.client })
  const endpointNew = await executeShellCommand(parseShellCommand("workflow endpoint new workflow-1 node-1 default"), context, { client: fake.client })
  const endpointAlias = await executeShellCommand(parseShellCommand("workflow endpoint alias endpoint-1 smoke"), context, { client: fake.client })
  const endpointBind = await executeShellCommand(parseShellCommand("workflow endpoint bind endpoint-1 node-1"), context, { client: fake.client })
  assert.equal(nodeAdd.ok, true)
  assert.deepEqual(nodeAdd.bindings, { node: "node-2" })
  assert.equal(nodeRemove.ok, true)
  assert.equal(edgeAdd.ok, true)
  assert.equal(edgeRemove.ok, true)
  assert.equal(endpointNew.ok, true)
  assert.equal(endpointAlias.ok, true)
  assert.equal(endpointBind.ok, true)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { AddWorkflowNode: { session_id: "session-1", workflow_ref: "workflow-1", agent_id: "agent-2" } },
    { RemoveWorkflowNode: { session_id: "session-1", workflow_ref: "workflow-1", node_id: "node-2" } },
    { AddWorkflowEdge: { session_id: "session-1", workflow_ref: "workflow-1", from_node_id: "node-1", to_node_id: "node-2" } },
    { RemoveWorkflowEdge: { session_id: "session-1", workflow_ref: "workflow-1", edge_id: "edge-1" } },
    { CreateWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", entry_node_id: "node-1", alias: "default" } },
    { AliasWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", alias: "smoke" } },
    { BindWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", entry_node_id: "node-1" } },
  ])
})
