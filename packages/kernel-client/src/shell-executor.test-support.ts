import type {
  AgentInstance,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowPublicationDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./kernel-types.js"

export function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
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

export function makeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
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

export function makeWorkflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
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

export function makeWorkflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
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

export function makeWorkflowPublication(overrides: Partial<WorkflowPublicationDefinition> = {}): WorkflowPublicationDefinition {
  return {
    id: "publication-1",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "public_qa",
    enabled: true,
    route: "/qa",
    methods: ["POST"],
    parser: { kind: "json" },
    created_by_user_id: "local",
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

export function makeWorkflowWatchdog(overrides: Partial<WorkflowWatchdogDefinition> = {}): WorkflowWatchdogDefinition {
  return {
    id: "watchdog-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    enabled: true,
    interval_seconds: 60,
    invocation_prompt: "Run it",
    policy: "skip",
    wakeups_executed: 0,
    next_run_at_ms: 0,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

export function fakeClient(handler: (request: Record<string, unknown>) => Record<string, unknown>) {
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
