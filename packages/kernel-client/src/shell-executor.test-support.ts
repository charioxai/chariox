import type {
  AgentInstance,
  DaemonHealthProjection,
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
    kind: "ingress",
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
    trigger: { kind: "interval", every_seconds: 60 },
    interval_seconds: 60,
    invocation_prompt: "Run it",
    overlap_policy: "skip",
    policy: "skip",
    max_runs: 100,
    runs_started: 0,
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

export function daemonHealth(overrides: Partial<DaemonHealthProjection> = {}): DaemonHealthProjection {
  const base: DaemonHealthProjection = {
    metadata: { projection_version: 1, last_event_id: 0, generated_at_ms: 0 },
    session_command_lanes: [],
    agent_command_lanes: [],
    workflow_command_lanes: [],
    provider_runtime_lanes: [],
    provider_run_actor: { enqueued_commands: 0, enqueue_rejections: 0 },
    process: { process_id: 1234, current_resident_set_bytes: 128, peak_resident_set_bytes: 256 },
    capability_executor: {
      max_concurrent_jobs: 64,
      available_permits: 64,
      submitted_jobs: 0,
      running_jobs: 0,
      completed_jobs: 0,
      failed_jobs: 0,
      rejected_jobs: 0,
      join_errors: 0,
    },
    session_projection: {
      projected_sessions: 1,
      projected_session_list_entries: 1,
      active_prompts: 0,
      queued_prompts: 0,
    },
    agent_runtime_projection: {
      projected_agents: 1,
      active_prompts: 0,
      queued_prompts: 0,
    },
    provider_catalog: {
      cached: true,
      expired: false,
      age_ms: 1000,
      ttl_ms: 60000,
    },
    provider_runs: {
      projected_runs: 0,
      active_runs: 0,
      arroba_active_runs: 0,
      native_tui_active_runs: 0,
      terminal_diagnostics: [],
      duplicate_arroba_agent_bindings: [],
      duplicate_native_tui_agent_bindings: [],
      multi_interface_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
    transport: {
      active_connections: 1,
      active_subscriptions: 1,
      retained_event_limit: 1000,
      command_result_cache_limit: 1000,
      inbound_request_limit: 100,
      incoming_requests: 0,
      emitted_events: 0,
      replay_gaps: 0,
      inbound_overload_rejections: 0,
      duplicate_command_conflicts: 0,
      outgoing_queue_overflows: 0,
      slow_consumer_closes: 0,
      relay_reconnect_attempts: 0,
      relay_last_reconnect_reason: null,
      relay_last_reconnect_delay_ms: null,
      relay_last_reconnect_url: null,
      relay_last_connected_url: null,
    },
    terminal_stream: {
      pending_output_records: 0,
      pending_notice_records: 0,
      pending_completion_records: 0,
      pending_output_record_limit_per_attachment: 4096,
      trimmed_pending_output_recipients: 0,
    },
    slice_lifecycle: {
      total_slices: 0,
      running_slices: 0,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 0,
      failed_operations: 0,
      in_progress_operations: 0,
      issues: [],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
    remote_execution: {
      remote_agents: 0,
      active_remote_agents: 0,
      missing_active_worker_runs: 0,
      malformed_bindings: 0,
      issues: [],
    },
    remote_extension_sync: {
      remote_agents: 0,
      home_proxy_agents: 0,
      home_proxy_grants: 0,
      manifest_missing_agents: 0,
      synced_agents: 0,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
      issues: [],
    },
    workspace_coordination: {
      active_worktree_claims: [],
      worktree_collisions: [],
      active_operation_claims: [],
    },
    workspace_live_sync: {
      active_reservations: 0,
      active_reservation_artifacts: 0,
      managed_mode: {
        write_fence_supported: true,
        write_fence_backend: "macos-seatbelt",
        unavailable_reason: null,
      },
      workspace_identity: {
        tracked_provider_runs: 0,
        identity_changed_provider_runs: 0,
        invalid_provider_runs: 0,
        current_generation_total: 0,
        issues: [],
      },
      external_changes: {
        tracked_artifacts: 0,
        externally_changed_artifacts: 0,
        external_change_events: 0,
        live_watcher_started: true,
        live_watcher_scans: 0,
        live_watcher_scan_errors: 0,
        issues: [],
      },
    },
    projection_invariants: {
      checked_sessions: 1,
      checked_agents: 1,
      mismatches: [],
    },
  }
  return { ...base, ...overrides }
}
