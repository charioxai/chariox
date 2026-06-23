import { execFileSync } from "node:child_process"

import { createCommandActionHandlers } from "./command-actions.js"
import type { AgentInstance, RuntimeSession } from "./cli-types.js"

export function runGit(cwd: string, args: string[]) {
  execFileSync("git", args, { cwd, stdio: "pipe" })
}

export function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
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

export function makeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
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

export function makeCommandDeps(overrides: Record<string, unknown> = {}) {
  let currentSession = makeSession()
  return {
    workspace: process.cwd(),
    worktree: process.cwd(),
    accountProfile: "default",
    clientId: "cli-1",
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
    flashFooter: () => {},
    appendNotice: () => {},
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
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
    updateSessionResponseLayout: async () => ({ session: currentSession, config: currentSession.config_state }),
    updateSessionConfig: async () => ({ session: currentSession, config: currentSession.config_state }),
    applySessionState: (session: RuntimeSession) => { currentSession = session },
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
    launchAgentProviderRun: async (provider: string, model: string, variant: string, agentId: string) => ({
      id: "provider-run-1",
      session_id: "session-1",
      agent_instance_id: agentId,
      adapter_key: provider,
      provider,
      account_profile: "default",
      model,
      variant,
      usage_tokens_total: null,
      state: "running",
    }),
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    undoTurn: async (agentRef?: string | null) => ({
      session_id: currentSession.id,
      agent_id: agentRef ?? currentSession.focused_agent_id ?? "agent-1",
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      provider_run_id: "provider-run-1",
      reverted_paths: ["src/lib.ts"],
      path_results: [{ path: "src/lib.ts", status: "applied", message: "restored" }],
    }),
    forkAgent: async (sourceAgentRef?: string | null) => {
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: "fork",
        state: "Focused",
      })
      currentSession = makeSession({ focused_agent_id: agent.id, agents: [...currentSession.agents, agent] })
      return {
        source_agent_id: sourceAgentRef ?? "agent-1",
        agent,
        provider_run: {
          id: "provider-run-2",
          session_id: "session-1",
          agent_instance_id: agent.id,
          adapter_key: agent.provider,
          provider: agent.provider,
          account_profile: "default",
          model: agent.model ?? "openai/gpt-5",
          variant: "medium",
          usage_tokens_total: null,
          state: "running",
        },
        session: currentSession,
      }
    },
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, _machineRef?: string) => {
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        state: "Focused",
      })
      currentSession = makeSession({ focused_agent_id: agent.id, agents: [...currentSession.agents, agent] })
      return { agent, session: currentSession }
    },
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: currentSession.agents[0] ?? makeAgent(), session: currentSession }),
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
    formatAgentLabel: (agent: AgentInstance | null | undefined) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
    ...overrides,
  } as Parameters<typeof createCommandActionHandlers>[0]
}
