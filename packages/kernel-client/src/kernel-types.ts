export type ArrobaMcpServerConfig = {
  name: string
  transport: Record<string, unknown>
  enabled?: boolean
  required?: boolean
  startup_timeout_sec?: number | null
  tool_timeout_sec?: number | null
  enabled_tools?: string[] | null
  disabled_tools?: string[] | null
  tools?: Record<string, unknown>
}

export type McpImportSkip = {
  name: string
  reason: string
}

export type McpImportOutcome = {
  imported: ArrobaMcpServerConfig[]
  skipped: McpImportSkip[]
}

export type ArrobaSkillMetadata = {
  name: string
  description: string
  short_description?: string | null
  path: string
}

export type SkillImportSkip = {
  name: string
  path: string
  reason: string
}

export type SkillImportOutcome = {
  imported: ArrobaSkillMetadata[]
  skipped: SkillImportSkip[]
}

export type RuntimeSession = {
  id: string
  alias?: string | null
  workspace_id: string
  worktree_id: string
  created_at_ms: number
  last_used_at_ms?: number | null
  status: string
  active_provider_run_id: string | null
  attachment_ids: string[]
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
  prompt_states?: Record<string, AgentPromptState>
  focused_agent_id: string | null
  max_agents: number
  agents: AgentInstance[]
  config_state: SessionConfigState
  workflow_launch_policy?: "reject" | "queue" | null
  workflows?: WorkflowDefinition[]
  workflow_runs?: WorkflowRun[]
  queued_workflow_launches?: QueuedWorkflowLaunch[]
  workflow_watchdogs?: WorkflowWatchdogDefinition[]
  workflow_consoles?: WorkflowConsole[]
}

export type AgentPromptState = {
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
}

export type SessionConfigState = {
  version: number
  values: Record<string, string>
  updated_by_attachment_id?: string | null
}

export type ArrobaUserConfig = {
  version: number
  providers?: {
    default?: string
    model?: string
    account_profile?: string
    effort?: string
    managed_io?: Record<string, "required" | "unrestricted">
  }
  history?: {
    operational?: {
      backend?: "sqlite"
      path?: string
      retention_days?: number
      max_size_mb?: number
      keep_pinned_sessions?: boolean
      archive_inactive_after_days?: number
      archive_deleted_agents?: boolean
    }
    archive?: {
      mode?: "disabled" | "external"
      url?: string
      token_env?: string
      archive_deleted_agents?: boolean
      archive_before_delete?: boolean
      delete_operational_after_verified_archive?: boolean
      require_durable_acceptance?: boolean
    }
  }
  state?: {
    backend?: "sqlite"
    path?: string
    snapshot_interval_events?: number
  }
  ui?: Record<string, unknown>
  relay?: Record<string, unknown>
  kernel?: Record<string, unknown>
}

export type ArrobaUserConfigPayload = {
  path: string
  config: ArrobaUserConfig
}

export type AgentInstance = {
  id: string
  agent_ref: string
  session_id: string
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  worktree_id: string | null
  remote_execution?: {
    worker_kernel_id: string
    worker_machine_id: string
    execution_lease_id: string
    leased_agent_id: string
  } | null
  mcp_grants?: string[]
  skill_grants?: string[]
  state: "Idle" | "Working" | "Focused" | "Error"
  is_processing: boolean
  grid_row: number
  grid_col: number
  grid_row_span: number
  grid_col_span: number
  created_at_ms: number
  last_activity_at_ms: number
}

export type PromptQueueItem = {
  id: string
  source_attachment_id: string
  target_agent_id?: string | null
  prompt: string
  status: string
}

export type RuntimeAttachment = {
  id: string
  session_id: string
}


export type RelayStatus = {
  configured: boolean
  connected: boolean
  relay_url?: string | null
  relay_token_configured: boolean
  daemon_id: string
  machine_id: string
  machine_alias?: string | null
}

export type RemoteMachineRecord = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name: string
  trust_status: "approved" | "pending" | "forgotten"
  online: boolean
  pending: boolean
  kernel_count: number
  available_providers?: string[]
}

export type PairedClientRecord = {
  client_id: string
  alias?: string | null
  public_key_thumbprint: string
  paired_at_ms: number
  revoked: boolean
}

export type PairingInviteIntent = "client" | "machine"

export type PairingInviteRecord = {
  intent: PairingInviteIntent
  invite_id: string
  invite_token: string
  relay_url: string
  target_daemon_id: string
  target_daemon_alias?: string | null
  issued_at_ms: number
  expires_at_ms: number
}

export type PairingJoinRecord = {
  intent: PairingInviteIntent
  subject_id: string
  relay_url: string
  target_daemon_id: string
  alias?: string | null
  public_key_thumbprint: string
  paired_at_ms: number
}

export type RelayKernelPresence = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  relay_alias?: string | null
  kernel_alias?: string | null
  available_providers?: string[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type RuntimeProviderRun = {
  id: string
  session_id: string
  agent_instance_id: string | null
  adapter_key: string
  provider: string
  account_profile: string
  model: string
  variant: string | null
  usage_tokens_total: number | null
  state: string
  started_at_ms?: number
  last_activity_at_ms?: number
  control_capabilities?: {
    operation: string
    mode: string
  }[]
}

export type ProviderProcessInfo = {
  process_id: string
  provider: string
  process_label: string
  pid?: number | null
  endpoint_mode: string
  status: "active" | "idle"
  started_at_ms: number
  last_activity_at_ms: number
  provider_session_ids: string[]
  owner_session_ids: string[]
  owner_provider_run_ids: string[]
  attached_session_ids: string[]
  active_workflow_run_ids: string[]
  teardown_safe: boolean
  teardown_blockers: string[]
}

export type ProviderAuthStatus = {
  provider: string
  auth_state: string
  account_profile: string | null
  login_hint: string | null
  detected_version: string | null
}

export type ProviderLoginStart = {
  provider: string
  login_kind: string
  login_id: string | null
  auth_url: string | null
  verification_url: string | null
  user_code: string | null
}

export type ProviderLogoutResult = {
  provider: string
}

export type PromptAttachmentPart = {
  url: string
  mime: string
  filename: string | null
}

export type StoredTransferArtifact = {
  artifact_id: string
  stored_path: string
  display_name: string
}

export type CaptureScreenshotResult = {
  status: string
  artifact_path: string | null
  message: string
}

export type RuntimeNoticeRecord = {
  message: string
}

export type TerminalOutputRecord = {
  agent_id?: string | null
  kind: "provider_output" | "prompt_echo" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status"
  merge_key?: string
  bytes: number[]
}

export type PromptSubmittedPayload = {
  outcome: Record<string, unknown>
  session: RuntimeSession
}

export type SessionHistoryPage = {
  entries: SessionHistoryPageEntry[]
  next_cursor: SessionHistoryCursor | null
}

export type SessionHistoryCursor = {
  before_entry_index: number
  before_entry_char_offset: number | null
}

export type SessionHistoryPageEntry = {
  entry_index: number
  fragment_start: number
  fragment_end: number
  total_chars: number
  entry: SessionHistoryEntry
}

export type SessionHistoryEntry = {
  agent_id?: string | null
  provider_run_id?: string | null
  kind: "user_prompt" | "provider_output" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status" | "notice"
  merge_key?: string
  text: string
}

export type HistoryEventKind =
  | "user_prompt"
  | "provider_output"
  | "provider_reasoning"
  | "provider_tool"
  | "provider_error"
  | "provider_status"
  | "notice"
  | "session_created"
  | "agent_created"
  | "agent_moved"
  | "workflow_started"
  | "workflow_node_started"
  | "workflow_node_completed"
  | "mcp_granted"
  | "skill_granted"
  | "remote_machine_connected"
  | "remote_machine_disconnected"
  | "git_commit_detected"
  | "git_worktree_changed"
  | "git_worktree_dirty"
  | "git_worktree_clean"
  | "git_push_detected"

export type HistoryEventRole = "user" | "assistant" | "tool" | "system"

export type HistoryAttributionConfidence = "definite" | "likely" | "ambiguous" | "unattributed"

export type HistoryEvent = {
  event_id: string
  sequence: number
  timestamp_ms: number
  workspace_id?: string | null
  session_id?: string | null
  agent_id?: string | null
  agent_alias?: string | null
  provider?: string | null
  model?: string | null
  turn_id?: string | null
  prompt_id?: string | null
  provider_run_id?: string | null
  provider_session_id?: string | null
  workflow_id?: string | null
  workflow_run_id?: string | null
  workflow_node_id?: string | null
  machine_id?: string | null
  repo_root?: string | null
  worktree_path?: string | null
  kind: HistoryEventKind
  role?: HistoryEventRole | null
  content?: string | null
  content_ref?: string | null
  metadata?: Record<string, unknown>
  candidate_agent_ids?: string[]
  candidate_prompt_ids?: string[]
  candidate_turn_ids?: string[]
  attribution_confidence?: HistoryAttributionConfidence | null
  caused_by_event_id?: string | null
}

export type HistoryQueryPayload = {
  session_id?: string | null
  agent_id?: string | null
  provider?: string | null
  model?: string | null
  workflow_id?: string | null
  machine_id?: string | null
  repo_root?: string | null
  worktree_path?: string | null
  kind?: HistoryEventKind | string | null
  text?: string | null
  after_sequence?: number | null
  limit?: number | null
}

export type HistoryEventsPayload = {
  events: HistoryEvent[]
  next_sequence: number | null
}

export type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "error" | "status" | "notice" | "turn_toggle"
  text: string
  sourceText?: string
  mergeKey?: string
  emphasis?: "muted" | "warning" | "error"
  turnId?: number
  hidden?: boolean
  toggleMode?: "expand" | "collapse"
  blobCollapsible?: boolean
  blobCollapsed?: boolean
  blobTitle?: string
  blobSummary?: string
  historyDeferred?: boolean
  historyEntryIndex?: number
  historyFragmentStart?: number
  historyFragmentEnd?: number
  historyTotalChars?: number
}

export type WorkflowDefinition = {
  id: string
  alias: string | null
  flush_agent_context_before_run?: boolean
  run_output_schema_ref?: string | null
  intermediate_output_schema_ref?: string | null
  nodes?: WorkflowNodeDefinition[]
  edges?: WorkflowEdgeDefinition[]
  endpoints?: WorkflowEndpointDefinition[]
}

export type WorkflowEndpointDefinition = {
  id: string
  alias: string | null
  entry_node_id: string
}

export type WorkflowWatchdogDefinition = {
  id: string
  workflow_id: string
  endpoint_id: string
  enabled: boolean
  interval_seconds: number
  invocation_prompt: string
  policy: "skip" | "queue"
  max_wakeups?: number | null
  wakeups_executed: number
  next_run_at_ms: number
  last_run_at_ms?: number | null
  last_status?: string | null
  last_error?: string | null
  last_workflow_run_id?: string | null
  pending_run?: boolean
  created_at_ms: number
  updated_at_ms: number
}

export type QueuedWorkflowLaunch = {
  id: string
  workflow_id: string
  endpoint_id: string
  invocation_prompt?: string | null
  source: "manual" | "watchdog"
  watchdog_id?: string | null
  queued_at_ms: number
}

export type WorkflowNodeDefinition = {
  id: string
  agent_id: string
  instructions?: string | null
  can_complete_workflow_run?: boolean
  can_emit_intermediate_run_output?: boolean
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowEdgeDefinition = {
  id: string
  from_node_id: string
  to_node_id: string
  output_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowMessage = {
  id: string
  source_node_run_id: string | null
  target_node_id: string
  message_type: string
  summary: string
  handoff_payload: string
  created_at_ms: number
}

export type WorkflowNodeRun = {
  id: string
  node_id: string
  agent_id: string
  status: string
  summary: string | null
  completion?: {
    summary: string
    output?: {
      message: string
    } | null
  } | null
  turn_envelope?: {
    delivery_token: string
    state: string
    rendered_prompt?: string | null
    mailbox_content?: string | null
    handoff_payloads_json?: string | null
    runtime_tool_calls?: {
      tool_name: string
      arguments_json: string
      result_json?: string | null
      ok: boolean
      timestamp_ms: number
    }[]
    prepared_at_ms: number
    dispatched_at_ms?: number | null
    acknowledged_at_ms?: number | null
    validated_completed_at_ms?: number | null
  } | null
  created_at_ms: number
  started_at_ms: number | null
  completed_at_ms: number | null
}

export type WorkflowFailureEvent = {
  kind: string
  source_node_run_id: string
  edge_ids: string[]
  message: string
  timestamp_ms: number
}

export type WorkflowRun = {
  id: string
  workflow_id: string
  endpoint_id: string
  entry_node_id: string
  status: string
  invocation_prompt: string | null
  active_node_run_id: string | null
  node_runs: WorkflowNodeRun[]
  messages: WorkflowMessage[]
  failure_events?: WorkflowFailureEvent[]
  intermediate_outputs?: {
    id: string
    source_node_run_id: string
    output: {
      message: string
    }
    valid: boolean
    warning?: string | null
    timestamp_ms: number
  }[]
  final_output?: {
    message: string
  } | null
  final_output_valid?: boolean | null
  final_output_warning?: string | null
  completed_by_node_run_id?: string | null
  created_at_ms: number
  started_at_ms: number | null
  completed_at_ms: number | null
}

export type WorkflowConsoleEntry = {
  timestamp_ms: number
  source_node_run_id?: string | null
  source_agent_id?: string | null
  text: string
}

export type WorkflowConsole = {
  workflow_id: string
  entries?: WorkflowConsoleEntry[]
}

export type ReadDirectoryTreeResult = {
  session_id: string
  root_path: string
  entries: unknown[]
}
