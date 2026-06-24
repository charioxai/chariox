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

export type ArrobaEnvironmentConfig = {
  name: string
  runtime: Record<string, unknown>
}

export type ArrobaScriptMetadata = {
  name: string
  runtime: "python" | "typescript" | string
  path: string
  description: string
  input_schema: Record<string, unknown>
  definition_hash: string
  timeout_sec?: number | null
}

export type ArrobaConnectorDefinition = {
  kind: "connector" | string
  name: string
  description: string
  adapter: string
  credential?: { required?: boolean } | null
  timeout_ms?: number | null
  max_response_bytes?: number | null
  operations: Array<Record<string, unknown>>
}

export type ArrobaConnectorAdapterDefinition = {
  kind: "connector_adapter" | string
  name: string
  version?: string | null
  adapter_protocol: string
  command: string
  args?: string[]
  description?: string | null
  source?: "user" | "bundled" | string | null
  manifest_path?: string | null
}

export type ArrobaCredentialConfig = {
  id: string
  description?: string | null
  source: Record<string, unknown>
  allowed_hosts?: string[]
  allowed_uses?: ("http" | "pty" | "connector" | "browser" | "mcp" | string)[]
  injection: Record<string, unknown>
}

export type ExtensionKind = "mcp" | "skill" | "script" | "connector"

export type ExtensionGrant = {
  kind: ExtensionKind
  name: string
  environment?: string | null
  credential?: string | null
  max_safety?: "read" | "write" | "destructive" | string | null
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

export type ProviderCapabilityImportDuplicate = {
  provider: string
  source: string
  hash?: string | null
  reason: string
}

export type ProviderCapabilityImportEntry = {
  kind: string
  name: string
  provider: string
  source: string
  hash?: string | null
  action: string
  reason: string
  duplicates?: ProviderCapabilityImportDuplicate[]
}

export type ProviderCapabilityImportSummary = {
  candidates: number
  imported: number
  updated: number
  already_installed: number
  deduped: number
  skipped: number
  errors: number
}

export type ProviderCapabilityImportReport = {
  dry_run: boolean
  providers: string[]
  summary: ProviderCapabilityImportSummary
  mcps: ProviderCapabilityImportEntry[]
  skills: ProviderCapabilityImportEntry[]
}

export type RuntimeSession = {
  id: string
  alias?: string | null
  workspace_id: string
  worktree_id: string
  owner_user_id?: string
  host_machine_id?: string | null
  host_daemon_id?: string | null
  members?: SessionMember[]
  invites?: SessionInvite[]
  created_at_ms: number
  last_used_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  status: string
  agent_defaults?: SessionAgentDefaults
  active_provider_run_id: string | null
  attachment_ids: string[]
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
  prompt_states?: Record<string, AgentPromptState>
  agent_activity?: Record<string, AgentRuntimeActivity>
  active_interactions?: RuntimeInteraction[]
  metaagent_tasks?: MetaagentTask[]
  focused_agent_id: string | null
  max_agents: number
  agents: AgentInstance[]
  collaboration_agent_counts?: SessionCollaborationAgentCounts | null
  config_state: SessionConfigState
  workflows?: WorkflowDefinition[]
  workflow_publications?: WorkflowPublicationDefinition[]
  workflow_runs?: WorkflowRun[]
  workflow_prompt_queues?: WorkflowPromptQueueDefinition[]
  workflow_queued_prompts?: WorkflowQueuedPrompt[]
  workflow_watchdogs?: WorkflowWatchdogDefinition[]
  workflow_consoles?: WorkflowConsole[]
  workspace_links?: WorkspaceLinkDefinition[]
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  external_provider_imports?: ExternalProviderImportMetadata[]
}

export type MetaagentTaskStatus = "active" | "paused" | "blocked" | "completed" | "aborted"

export type MetaagentTask = {
  task_id: string
  metaagent_id: string
  status: MetaagentTaskStatus
  task_markdown: string
  plan_markdown: string
  revision: number
  created_at_ms: number
  updated_at_ms: number
  completed_at_ms?: number | null
  blocked_reason?: string | null
  aborted_reason?: string | null
  completion_summary?: string | null
}

export type ExternalProviderObservedCursor = {
  last_observed_turn_id?: string | null
  last_observed_at_ms?: number | null
  last_observed_merge_key?: string | null
}

export type ExternalProviderImportMetadata = {
  external_provider_session_id: string
  external_provider: string
  external_provider_session_provider_id: string
  observed_cursor: ExternalProviderObservedCursor
  last_observed_turn_id?: string | null
  last_observed_at_ms?: number | null
  imported_at_ms: number
}

export type SessionCollaborationAgentCounts = {
  owned_agent_count: number
  other_user_agent_count: number
  total_agent_count: number
  collaborator_count: number
}

export type SessionAgentDefaults = {
  provider: string
  model?: string | null
  effort?: string | null
  account_profile?: string | null
  execution_mode?: "build" | "plan" | null
  permission_level?: "required" | "yolo" | null
}

export type WaitingRoomPublicSessionSummary = {
  id: string
  alias?: string | null
  workspace_id: string
  worktree_id: string
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  created_at_ms: number
  last_used_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  status: string
  connected_cli_count: number
  activity?: WaitingRoomSessionActivitySummary
  agents?: WaitingRoomPublicAgentSummary[]
  workflows?: WaitingRoomPublicWorkflowSummary[]
}

export type WaitingRoomSessionActivitySummary = {
  agent_count: number
  working_agent_count: number
  active_prompt_count: number
  queued_prompt_count: number
  error_agent_count: number
  remote_agent_count?: number
  missing_worker_provider_run_count?: number
  home_proxy_agent_count?: number
  remote_extension_sync_issue_count?: number
  remote_extension_pending_revoke_count?: number
  unread_idle_agent_count?: number
}

export type WaitingRoomPublicItemActivitySummary = {
  working: boolean
  active_prompt_count: number
  queued_prompt_count: number
  error: boolean
  unread_idle_output?: boolean
}

export type WaitingRoomPublicAgentSummary = {
  id: string
  agent_ref: string
  alias?: string | null
  created_at_ms: number
  last_prompt_sent_at_ms?: number | null
  provider: string
  model?: string | null
  variant?: string | null
  permission?: string | null
  workspace_id: string
  worktree_id: string
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  extension_grants?: ExtensionGrant[]
  activity?: WaitingRoomPublicItemActivitySummary
}

export type WaitingRoomPublicWorkflowSummary = {
  id: string
  alias?: string | null
  created_at_ms: number
  activity?: WaitingRoomPublicItemActivitySummary
  nodes?: WaitingRoomPublicWorkflowNodeSummary[]
  edges?: WaitingRoomPublicWorkflowEdgeSummary[]
  endpoints?: WaitingRoomPublicWorkflowEndpointSummary[]
}

export type WaitingRoomPublicWorkflowNodeSummary = {
  id: string
  agent_id: string
  label: string
  wait_for_all_inputs?: boolean
}

export type WaitingRoomPublicWorkflowEdgeSummary = {
  id: string
  from_node_id: string
  to_node_id: string
}

export type WaitingRoomPublicWorkflowEndpointSummary = {
  id: string
  alias?: string | null
  entry_node_id: string
}

export type WorkspaceLinkAttachment = {
  link_id: string
  user_id: string
  machine_id: string
  kernel_id: string
  repo_root: string
  branch?: string | null
  repo_fingerprint?: string | null
  attached_at_ms: number
}

export type WorkspaceLinkDefinition = {
  link_id: string
  session_id: string
  name: string
  created_by_user_id: string
  created_at_ms: number
  attachments?: WorkspaceLinkAttachment[]
}

export type WorkspaceLiveSyncFooterState = "off" | "managed" | "tracked" | "syncing" | "conflict" | "degraded"

export type WorkspaceLiveSyncTargetState = "ready" | "degraded" | "conflict"

export type WorkspaceLiveSyncTargetStatus = {
  link_id: string
  link_name: string
  user_id: string
  machine_id: string
  kernel_id: string
  repo_root: string
  branch?: string | null
  repo_fingerprint?: string | null
  status: WorkspaceLiveSyncTargetState
  attached_at_ms: number
}

export type WorkspaceLiveSyncGroupStatus = {
  group_id: string
  group_name: string
  target_count: number
  ready_targets: number
  degraded_targets: number
  conflicted_targets: number
}

export type WorkspaceLiveSyncConflictSummary = {
  conflict_id: string
  link_id: string
  source_agent_id: string
  target_user_id: string
  target_repo_root: string
  path: string
  next_action: string
}

export type WorkspaceLiveSyncIgnoreStatus = {
  ignore_file?: string | null
  rules: string[]
  force_excludes: string[]
}

export type WorkspaceLiveSyncStatus = {
  session_id: string
  mode: "managed" | "tracked" | "unrestricted"
  footer_state: WorkspaceLiveSyncFooterState
  sync_groups: WorkspaceLiveSyncGroupStatus[]
  targets: WorkspaceLiveSyncTargetStatus[]
  conflicts: WorkspaceLiveSyncConflictSummary[]
  ignore: WorkspaceLiveSyncIgnoreStatus
}

export type ProjectionMetadata = {
  projection_version: number
  last_event_id: number
  generated_at_ms: number
}

export type ActorQueueSnapshot = {
  lane_id: string
  queue_limit: number
  queued_commands: number
}

export type SessionProjectionHealthSnapshot = {
  projected_sessions: number
  projected_session_list_entries?: number | null
  active_prompts: number
  queued_prompts: number
}

export type AgentRuntimeProjectionHealthSnapshot = {
  projected_agents: number
  active_prompts: number
  queued_prompts: number
}

export type ProviderRunActorHealthSnapshot = {
  enqueued_commands: number
  enqueue_rejections: number
}

export type CapabilityExecutorHealthSnapshot = {
  max_concurrent_jobs: number
  available_permits: number
  submitted_jobs: number
  running_jobs: number
  completed_jobs: number
  failed_jobs: number
  rejected_jobs: number
  join_errors: number
}

export type ProviderCatalogHealthSnapshot = {
  cached: boolean
  expired: boolean
  age_ms?: number | null
  ttl_ms: number
}

export type ProviderRunAgentBindingConflict = {
  session_id: string
  agent_id: string
  provider_run_ids: string[]
}

export type ProviderRunIdentityIssue = {
  provider_run_id: string
  session_id: string
  agent_id?: string | null
  details: string
}

export type ProviderRunSessionPointerIssue = {
  session_id: string
  active_provider_run_id?: string | null
  details: string
}

export type ProviderRunTerminalDiagnosticIssue = {
  provider_run_id: string
  session_id: string
  agent_id?: string | null
  provider: string
  state: string
  diagnostic: string
}

export type ProviderRunHealthSnapshot = {
  projected_runs: number
  active_runs: number
  arroba_active_runs: number
  native_tui_active_runs: number
  terminal_diagnostics: ProviderRunTerminalDiagnosticIssue[]
  duplicate_arroba_agent_bindings: ProviderRunAgentBindingConflict[]
  duplicate_native_tui_agent_bindings: ProviderRunAgentBindingConflict[]
  multi_interface_agent_bindings: ProviderRunAgentBindingConflict[]
  orphaned_active_runs: ProviderRunIdentityIssue[]
  session_active_run_mismatches: ProviderRunSessionPointerIssue[]
}

export type TransportHealthSnapshot = {
  active_connections: number
  active_subscriptions: number
  retained_event_limit: number
  command_result_cache_limit: number
  inbound_request_limit: number
  incoming_requests: number
  emitted_events: number
  replay_gaps: number
  inbound_overload_rejections: number
  duplicate_command_conflicts: number
  outgoing_queue_overflows: number
  slow_consumer_closes: number
  relay_reconnect_attempts: number
  relay_last_reconnect_reason: string | null
  relay_last_reconnect_delay_ms: number | null
  relay_last_reconnect_url: string | null
  relay_last_connected_url: string | null
}

export type TerminalStreamHealthSnapshot = {
  pending_output_records: number
  pending_notice_records: number
  pending_completion_records: number
  pending_output_record_limit_per_attachment: number
  trimmed_pending_output_recipients: number
}

export type WorktreeClaimSnapshot = {
  workspace_id: string
  worktree_id: string
  session_ids: string[]
}

export type WorkspaceOperationClaimSnapshot = {
  claim_id: string
  workspace_id: string
  worktree_id: string
  session_id: string
  attachment_id?: string | null
  operation: string
  mode: "read" | "write"
}

export type WorkspaceCoordinationHealthSnapshot = {
  active_worktree_claims: WorktreeClaimSnapshot[]
  worktree_collisions: WorktreeClaimSnapshot[]
  active_operation_claims: WorkspaceOperationClaimSnapshot[]
}

export type WorkspaceIdentityMonitorHealthSnapshot = {
  tracked_provider_runs: number
  identity_changed_provider_runs: number
  invalid_provider_runs: number
  current_generation_total: number
  issues: WorkspaceIdentityIssue[]
}

export type WorkspaceIdentityIssue = {
  provider_run_id: string
  root: string
  generation: number
  valid: boolean
  baseline_fingerprint: string
  current_fingerprint: string
  baseline_branch?: string | null
  current_branch?: string | null
  baseline_head_commit?: string | null
  current_head_commit?: string | null
  baseline_repo_url?: string | null
  current_repo_url?: string | null
}

export type ArtifactExternalChangeHealthSnapshot = {
  tracked_artifacts: number
  externally_changed_artifacts: number
  external_change_events: number
  live_watcher_started: boolean
  live_watcher_scans: number
  live_watcher_scan_errors: number
  issues: ArtifactExternalChangeIssue[]
}

export type ArtifactExternalChangeIssue = {
  artifact_key: string
  provider_run_id?: string | null
  workspace_fingerprint: string
  workspace_root?: string | null
  path: string
}

export type WorkspaceLiveSyncHealthSnapshot = {
  active_reservations: number
  active_reservation_artifacts: number
  managed_mode: {
    write_fence_supported: boolean
    write_fence_backend?: string | null
    unavailable_reason?: string | null
  }
  workspace_identity: WorkspaceIdentityMonitorHealthSnapshot
  external_changes: ArtifactExternalChangeHealthSnapshot
}

export type SliceLifecycleHealthSnapshot = {
  total_slices: number
  running_slices: number
  starting_slices: number
  stopping_slices: number
  stopped_slices: number
  unhealthy_slices: number
  attached_agents: number
  failed_operations: number
  in_progress_operations: number
  issues: SliceLifecycleIssue[]
  provider_auth_missing_slices: number
  provider_auth_unconfigured_slices: number
  provider_auth_issues: SliceProviderAuthIssue[]
}

export type SliceLifecycleIssue = {
  slice_id: string
  name: string
  status: string
  last_operation?: string | null
  last_operation_status?: string | null
  last_error?: string | null
  session_ids: string[]
  agent_ids: string[]
  worktree_id?: string | null
}

export type SliceProviderAuthIssue = {
  slice_id: string
  name: string
  status: string
  session_ids: string[]
  agent_ids: string[]
  worktree_id?: string | null
  provider?: string | null
  provider_auth_state?: string | null
  alias?: string | null
  identity?: string | null
  details: string
}

export type RemoteExtensionSyncHealthSnapshot = {
  remote_agents: number
  home_proxy_agents: number
  home_proxy_grants: number
  manifest_missing_agents: number
  synced_agents: number
  syncing_agents: number
  pending_agents: number
  failed_agents: number
  stale_agents: number
  pending_revoke_agents: number
  issues: RemoteExtensionSyncIssue[]
}

export type RemoteExtensionSyncIssue = {
  session_id: string
  agent_id: string
  agent_ref: string
  worker_kernel_id: string
  worker_machine_id: string
  execution_lease_id: string
  leased_agent_id: string
  active_worker_provider_run_id?: string | null
  state: string
  manifest_hash?: string | null
  last_error?: string | null
  pending_revoke: boolean
  home_proxy_grants: string[]
  worktree_id?: string | null
}

export type RemoteExecutionHealthSnapshot = {
  remote_agents: number
  active_remote_agents: number
  missing_active_worker_runs: number
  malformed_bindings: number
  issues: RemoteExecutionIssue[]
}

export type RemoteExecutionIssue = {
  kind: string
  session_id: string
  agent_id: string
  agent_ref: string
  worker_kernel_id: string
  worker_machine_id: string
  execution_lease_id: string
  leased_agent_id: string
  active_worker_provider_run_id?: string | null
  state: string
  is_processing: boolean
  worktree_id?: string | null
  details: string
}

export type ProjectionInvariantMismatch = {
  kind: string
  session_id: string
  agent_id?: string | null
  details: string
}

export type ProjectionInvariantHealthSnapshot = {
  checked_sessions: number
  checked_agents: number
  mismatches: ProjectionInvariantMismatch[]
}

export type DaemonHealthProjection = {
  metadata: ProjectionMetadata
  session_command_lanes: ActorQueueSnapshot[]
  agent_command_lanes: ActorQueueSnapshot[]
  workflow_command_lanes: ActorQueueSnapshot[]
  provider_runtime_lanes: ActorQueueSnapshot[]
  provider_run_actor: ProviderRunActorHealthSnapshot
  process: {
    process_id: number
    current_resident_set_bytes?: number | null
    peak_resident_set_bytes?: number | null
  }
  capability_executor: CapabilityExecutorHealthSnapshot
  session_projection: SessionProjectionHealthSnapshot
  agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot
  provider_catalog: ProviderCatalogHealthSnapshot
  provider_runs: ProviderRunHealthSnapshot
  transport: TransportHealthSnapshot
  terminal_stream: TerminalStreamHealthSnapshot
  slice_lifecycle: SliceLifecycleHealthSnapshot
  remote_execution: RemoteExecutionHealthSnapshot
  remote_extension_sync: RemoteExtensionSyncHealthSnapshot
  workspace_coordination: WorkspaceCoordinationHealthSnapshot
  workspace_live_sync: WorkspaceLiveSyncHealthSnapshot
  projection_invariants: ProjectionInvariantHealthSnapshot
}

export type DaemonHealthResponse = {
  DaemonHealth: {
    projection: DaemonHealthProjection
  }
}

export type SessionMember = {
  user_id: string
  joined_at_ms: number
  invited_by_user_id?: string | null
  collaboration_level?: CollaborationLevel
}

export type SessionInvite = {
  invite_id: string
  session_id: string
  created_by_user_id: string
  created_at_ms: number
  expires_at_ms?: number | null
  max_uses?: number | null
  used_count: number
  revoked_at_ms?: number | null
  collaboration_level?: CollaborationLevel
}

export type CollaborationLevel = "private" | "transparent" | "full"

export type AgentPromptState = {
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
}

export type AgentRuntimeActivity = {
  status: "idle" | "working" | "error"
  prompt_status: "none" | "queued" | "running" | "cancelling" | "settling"
  busy: boolean
  unread_idle_output?: boolean
  active_turn?: AgentActiveTurn | null
  last_completed_turn?: CompletedGitTurnActionProjection | null
}

export type AgentActiveTurn = {
  prompt_id: string
  provider_run_id?: string | null
  status: "none" | "queued" | "running" | "cancelling" | "settling"
  phase: "accepted" | "awaiting_first_output" | "streaming" | "settling"
  started_at_ms?: number | null
}

export type CompletedGitTurnActionProjection = {
  turn_id: string
  prompt_id: string
  provider_run_id: string
  agent_id: string
  completed_at_ms: number
  duration_ms?: number | null
  changed_paths: string[]
  undo_available: boolean
  undo_unavailable_reason?: string | null
}

export type WorkspaceLiveSyncApplyStatus = "applied" | "rebased" | "skipped_conflict" | "failed_io"

export type WorkspaceLiveSyncPathApplyResult = {
  path: string
  status: WorkspaceLiveSyncApplyStatus
  message: string
}

export type TurnUndoResult = {
  session_id: string
  agent_id: string
  turn_id: string
  prompt_id: string
  provider_run_id: string
  reverted_paths: string[]
  path_results: WorkspaceLiveSyncPathApplyResult[]
}

export type AgentForkPayload = {
  source_agent_id: string
  agent: AgentInstance
  provider_run: RuntimeProviderRun
  session: RuntimeSession
}

export type RuntimeInteraction = {
  id: string
  agent_id: string
  kind: "choice" | "permission"
  level: "info" | "warning" | "critical"
  title?: string | null
  message: string
  choices: RuntimeInteractionChoice[]
  custom_choice?: RuntimeInteractionCustomChoice | null
  timeout_sec?: number | null
  default_on_timeout?: string | null
  requested_at_ms: number
}

export type RuntimeInteractionChoice = {
  id: string
  label: string
  reply: string
  style?: "primary" | "secondary" | "danger" | null
}

export type RuntimeInteractionCustomChoice = {
  id: string
  label: string
  placeholder?: string | null
  min_length?: number | null
  max_length?: number | null
  input_kind?: "text" | "secret" | null
}

export type RequestNativeProviderInteractionRequest = {
  session_id: string
  agent_id: string
  interaction_id: string
  level: "info" | "warning" | "critical"
  title?: string | null
  message: string
  choices: RuntimeInteractionChoice[]
  custom_choice?: RuntimeInteractionCustomChoice | null
  timeout_sec?: number | null
  default_on_timeout?: string | null
}

export type NativeProviderInteractionResolution = {
  status: string
  choice_id?: string | null
  reply?: string | null
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
    workspace_live_sync?: "off" | "managed" | "tracked" | "unrestricted"
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
  credential_vault?: {
    backend?: "arroba_encrypted" | "process_memory"
    service?: string
    path?: string
    unlock_policy?: "kernel_init" | "ttl" | "always"
    default_ttl_minutes?: number
    max_ttl_minutes?: number
  }
}

export type ArrobaUserConfigPayload = {
  path: string
  config: ArrobaUserConfig
  effects?: UserConfigMutationEffect[]
}

export type ArrobaUserConfigSchemaPayload = {
  entries: UserConfigSchemaEntry[]
}

export type UserConfigSchemaEntry = {
  path: string
  value_type: string
  allowed_values?: string[]
  settable: boolean
  unsettable: boolean
  effect: string
  status: string
  description: string
}

export type UserConfigMutationEffect = {
  kind: string
  path: string
  message: string
  provider_reload?: {
    reloaded: number
    deferred: number
    unaffected: number
  } | null
}

export type AgentInstance = {
  id: string
  agent_ref: string
  session_id: string
  role?: "standard" | "meta" | string
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  primary_provider?: string | null
  primary_model?: string | null
  primary_effort?: string | null
  execution_mode_override?: "build" | "plan" | null
  permission_level_override?: "required" | "yolo" | null
  workspace_id?: string | null
  worktree_id: string | null
  remote_execution?: {
    worker_kernel_id: string
    worker_machine_id: string
    execution_lease_id: string
    leased_agent_id: string
    active_worker_provider_run_id?: string | null
  } | null
  extension_grants?: ExtensionGrant[]
  remote_extension_manifest_sync?: RemoteExtensionManifestSyncStatus | null
  substitutes?: AgentSubstituteProfile[]
  active_substitute_index?: number | null
  last_substitution?: AgentSubstitutionRecord | null
  substitution_timeout_ms?: number | null
  visible_in_freeform?: boolean
  external_provider_import?: ExternalProviderImportMetadata | null
  state: "Idle" | "Working" | "Focused" | "Error"
  is_processing: boolean
  grid_row: number
  grid_col: number
  grid_row_span: number
  grid_col_span: number
  created_at_ms: number
  last_activity_at_ms: number
}

export type RemoteExtensionManifestSyncStatus = {
  state: "synced" | "syncing" | "pending" | "failed" | "stale"
  manifest_hash?: string | null
  last_attempted_at_ms?: number | null
  last_synced_at_ms?: number | null
  last_error?: string | null
  pending_revoke?: boolean | null
}

export type AgentSubstituteProfile = {
  provider: string
  model: string
  variant?: string | null
  kernel_id?: string | null
  worktree_id?: string | null
}

export type AgentSubstitutionRecord = {
  substitute_index: number
  reason: string
  activated_at_ms: number
}

export type PromptQueueItem = {
  id: string
  source_attachment_id: string
  target_agent_id?: string | null
  prompt: string
  attachments?: PromptAttachmentPart[]
  status: string
  prompt_origin?: "arroba" | "external" | string
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

export type CloudRelayProfile = {
  api_url: string
  email: string
  account_id: string
  user_id: string
  account_slug: string
  realm_id: string
  relay_url: string
  issuer_id: string
  client_id?: string | null
  client_alias?: string | null
  machine_id?: string | null
  machine_alias?: string | null
  machine_credential?: string | null
  cloud_session_token?: string | null
  cloud_session_expires_at_ms?: number | null
  token_expires_at_ms?: number | null
}

export type CloudRelayLoginStart = {
  api_url: string
  device_code: string
  user_code: string
  verification_url: string
  expires_at: string
  interval_seconds: number
}

export type CloudRelayLoginPoll = {
  status: "authorization_pending" | "expired_token" | "approved"
  interval_seconds?: number | null
  expires_at?: string | null
  profile?: CloudRelayProfile | null
}

export type CloudSessionInvite = {
  invite_id: string
  invite_token: string
  session_id: string
  account_id: string
  created_by_user_id: string
  expires_at?: string | null
  max_uses?: number | null
}

export type CloudSessionInviteDetails = {
  invite_id: string
  session_id: string
  account_id: string
  created_by_user_id: string
  display_name?: string | null
  expires_at?: string | null
  max_uses?: number | null
  used_count: number
  status: string
}

export type CloudSessionInviteAcceptance = {
  session_id: string
  account_id: string
  user_id: string
  invited_by_user_id: string
  joined_at: string
}

export type CloudSessionMember = {
  user_id: string
  email: string
  display_name?: string | null
  invited_by_user_id?: string | null
  joined_at: string
}

export type CloudCollaborator = {
  user_id: string
  email: string
  display_name?: string | null
  last_collaborated_at: string
  shared_session_count: number
}

export type CloudRelayRuntimeToken = {
  relay_url: string
  relay_token: string
  token_expires_at: string
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
  provider_accounts?: ProviderAccountSummary[]
}

export type SliceRecord = {
  id: string
  name: string
  owner_kernel_id: string
  owner_machine_id: string
  session_id?: string | null
  session_ids?: string[]
  agent_ids?: string[]
  backend: "local_docker" | "ssh_docker"
  os: string
  display_mode?: "headless" | "headed"
  status: "stopped" | "starting" | "stopping" | "running" | "unhealthy"
  last_operation?: string | null
  last_operation_status?: "accepted" | "in_progress" | "completed" | "failed" | "reconciled" | null
  last_error?: string | null
  last_operation_at_ms?: number | null
  workspace_id?: string | null
  worktree_id?: string | null
  workspace_mount?: string | null
  worker_kernel_ref: string
  worker_kernel_id?: string | null
  worker_machine_id?: string | null
  relay_endpoint?: SliceRelayEndpoint | null
  local_docker_ports?: SliceLocalDockerPorts | null
  providers?: string[]
  provider_auth?: SliceProviderAuthSummary[]
  saved_state_ref?: string | null
  saved_state_status?: "saved" | "missing" | "failed" | null
  saved_state_updated_at_ms?: number | null
  display_endpoint?: SliceDisplayEndpoint | null
  created_at_ms: number
  updated_at_ms: number
}

export type SliceSavedStateRecord = {
  id: string
  slice_name: string
  source_slice_id: string
  backend: "local_docker" | "ssh_docker"
  os: string
  image_ref: string
  home_archive_path: string
  created_at_ms: number
  updated_at_ms: number
  last_operation?: string | null
  last_operation_status?: "accepted" | "in_progress" | "completed" | "failed" | "reconciled" | null
  last_error?: string | null
}

export type SliceBackupRecord = {
  id: string
  name: string
  source_slice_id: string
  source_state_id: string
  image_ref: string
  home_archive_path: string
  created_at_ms: number
  size_bytes?: number | null
}

export type SliceLocalDockerPorts = {
  codex: number
  opencode: number
  kernel: number
  mcp: number
  relay: number
  novnc: number
  codex_range_start: number
  opencode_range_start: number
}

export type SliceLogEntry = {
  source: string
  path?: string | null
  text: string
  truncated?: boolean
}

export type SliceProviderAuthSummary = {
  provider: string
  state: "unknown" | "not_configured" | "configured" | "authenticated"
  auth_type?: string | null
  account_id?: string | null
  email?: string | null
  organization_id?: string | null
  organization_name?: string | null
  subscription_type?: string | null
  alias?: string | null
  source: string
}

export type SliceRelayEndpoint = {
  url: string
  private?: boolean
}

export type SliceDisplayEndpoint = {
  slice_id: string
  kind: "novnc" | "arroba_viewer" | "external"
  url: string
  access: "local" | "tunnel" | "public"
  expires_at_ms?: number | null
  capabilities?: string[]
}

export type PairedClientRecord = {
  client_id: string
  alias?: string | null
  terminal_type?: TerminalType | null
  public_key_thumbprint: string
  paired_at_ms: number
  revoked: boolean
}

export type PairingInviteIntent = "client" | "machine"
export type TerminalType = "cli" | "web" | "ios" | "android"

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

export type TerminalRecord = {
  terminal_id: string
  terminal_type: TerminalType
  alias?: string | null
  paired_at_ms: number
  revoked: boolean
}

export type TerminalPairingLinkRecord = {
  terminal_id: string
  pairing_link: string
  pairing_code: string
  invite_id: string
  relay_url: string
  target_daemon_id: string
  target_daemon_alias?: string | null
  terminal_type: TerminalType
  issued_at_ms: number
  expires_at_ms: number
}

export type RelayKernelPresence = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  relay_alias?: string | null
  kernel_alias?: string | null
  available_providers?: string[]
  provider_accounts?: ProviderAccountSummary[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type ProviderAccountSummary = {
  provider: string
  state: string
  auth_type?: string | null
  account_id?: string | null
  email?: string | null
  organization_id?: string | null
  organization_name?: string | null
  subscription_type?: string | null
  alias?: string | null
}

export type WaitingRoomInventorySnapshot = {
  inventory_version: string
  sessions: WaitingRoomPublicSessionSummary[]
  relay_status: RelayStatus
  terminals?: TerminalRecord[]
  launch_target?: {
    workspace_id: string
    worktree_id: string
    workspace_label?: string | null
    directory?: string | null
    worktree_label?: string | null
  } | null
}

export type WaitingRoomPublicSnapshot = WaitingRoomInventorySnapshot & {
  schema_version: number
  generated_at_ms: number
}

export type WorkspaceWorktreeRecord = {
  path: string
  branch?: string | null
  label?: string | null
  current: boolean
}

export type WorkspaceGitCompareRef = {
  name: string
  detail?: string | null
  selected: boolean
}

export type WorkspaceGitFileChange = {
  path: string
  status: string
  additions: number
  deletions: number
}

export type WorkspaceGitChangeTotals = {
  files: number
  additions: number
  deletions: number
}

export type WorkspaceGitOverview = {
  workspace_id: string
  worktree_id: string
  repo_root?: string | null
  repo_label?: string | null
  branch?: string | null
  compare_ref: string
  compare_refs: WorkspaceGitCompareRef[]
  totals: WorkspaceGitChangeTotals
  files: WorkspaceGitFileChange[]
  generated_at_ms: number
}

export type WorkspaceRepoFileEntry = {
  path: string
  name: string
  kind: "directory" | "file" | string
  changed: boolean
  status?: string | null
  additions: number
  deletions: number
}

export type WorkspaceRepoFileListing = {
  workspace_id: string
  worktree_id: string
  path_prefix: string
  compare_ref: string
  total_entries: number
  truncated: boolean
  entries: WorkspaceRepoFileEntry[]
  generated_at_ms: number
}

export type WorkspaceFileContent = {
  workspace_id: string
  worktree_id: string
  path: string
  name: string
  language: string
  mime: string
  encoding: "utf-8" | "base64" | string
  content_text?: string | null
  content_base64?: string | null
  size_bytes: number
  mtime_ms: number
  fingerprint: string
  sha256?: string | null
  truncated: boolean
  status?: string | null
  additions: number
  deletions: number
  compare_ref: string
  generated_at_ms: number
}

export type WorkspaceGitActionResult = {
  workspace_id: string
  worktree_id: string
  action: string
  message: string
  commit_sha?: string | null
  branch?: string | null
  generated_at_ms: number
}

export type WorkspacePullRequestRecord = {
  workspace_id: string
  worktree_id: string
  branch: string
  base_ref: string
  url: string
  title?: string | null
  draft: boolean
  generated_at_ms: number
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
  usage?: {
    total_tokens?: number | null
    last_tokens?: number | null
    context_tokens?: number | null
    context_window?: number | null
  }
  state: string
  endpoint_mode?: string
  client_interface?: "arroba" | "native_tui" | string
  process_label?: string
  structured_endpoint?: string | null
  provider_session_id?: string | null
  working_directory?: string | null
  started_at_ms?: number
  last_activity_at_ms?: number
  control_capabilities?: {
    operation: string
    mode: string
  }[]
  external_provider_import?: ExternalProviderImportMetadata | null
}

export const LOCAL_DAEMON_PROTOCOL_VERSION = 166

export type DebugBundleExportedResponse = {
  DebugBundleExported: {
    bundle_dir: string
    manifest_path: string
    logs_path: string
    log_root: string
    record_count: number
    limit: number
  }
}

export type AgentUtilityKind = "WorkspaceCommitMessage"

export type WorkspaceCommitMessageUtilityInput = {
  workspace_id: string
  worktree_id: string
  compare_ref?: string | null
}

export type AgentUtilityInput = {
  WorkspaceCommitMessage: WorkspaceCommitMessageUtilityInput
}

export type RunAgentUtilityRequest = {
  session_id: string
  agent_id: string
  kind: AgentUtilityKind
  input: AgentUtilityInput
}

export type AgentUtilityOutput = {
  WorkspaceCommitMessage: {
    message: string
  }
}

export type AgentUtilityResult = {
  utility_run_id: string
  session_id: string
  agent_id: string
  kind: AgentUtilityKind
  output: AgentUtilityOutput
  generated_at_ms: number
}

export type ProviderProcessInfo = {
  process_id: string
  provider: string
  process_label: string
  pid?: number | null
  resident_set_bytes?: number | null
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

export type SliceProviderLoginStart = {
  provider: string
  login_kind: string
  auth_url?: string | null
  verification_url?: string | null
  user_code?: string | null
  status: string
  message: string
}

export type ProviderLogoutResult = {
  provider: string
}

export type PromptAttachmentPart = {
  url: string
  mime: string
  filename: string | null
  contents_base64?: string | null
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
  agent_activity: Record<string, AgentRuntimeActivity>
  agent_activity_revision: number
}

export type QueuedPromptSteeredPayload = {
  prompt: PromptQueueItem
  session: RuntimeSession
  agent_activity: Record<string, AgentRuntimeActivity>
  agent_activity_revision: number
}

export type QueuedPromptCancelledPayload = {
  prompt: PromptQueueItem
  session: RuntimeSession
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
  source?: "external_provider_observed" | null
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
  observed_at_ms?: number | null
  attachments?: SessionHistoryPromptAttachment[]
  text: string
  timestamp_ms?: number
  source_attachment_id?: string | null
}

export type SessionHistoryPromptAttachment = {
  url: string
  mime: string
  filename?: string | null
  preview_url?: string | null
}

export type SessionHistoryOutline = {
  agents: SessionHistoryOutlineAgent[]
}

export type SessionHistoryOutlineAgent = {
  agent_id: string
  turns: SessionHistoryOutlineTurn[]
  next_cursor?: SessionHistoryOutlineCursor | null
}

export type SessionHistoryOutlineCursor = {
  before_sequence: number
}

export type SessionHistoryOutlineTurn = {
  turn_id: string
  prompt_id?: string | null
  started_at_ms: number
  user_prompt: SessionHistoryPageEntry
  entries: SessionHistoryPageEntry[]
  summary?: SessionHistoryPageEntry | null
  blobs: SessionHistoryOutlineBlob[]
}

export type SessionHistoryOutlineBlob = {
  blob_id: string
  kind: SessionHistoryEntry["kind"]
  title: string
  summary: string
  sequence_start: number
  sequence_end: number
  entry_count: number
  total_chars: number
  timestamp_ms: number
}

export type SessionHistoryBlobContent = {
  blob_id: string
  entries: SessionHistoryPageEntry[]
}

export type PromptInputHistoryEntryKind = "prompt" | "command"

export type PromptInputHistoryEntry = {
  sequence: number
  timestamp_ms: number
  session_id: string
  source_attachment_id?: string | null
  kind: PromptInputHistoryEntryKind
  text: string
}

export type PromptInputHistoryPage = {
  entries: PromptInputHistoryEntry[]
}

export type RecallEventKind =
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
  | "workspace_live_sync_mode_changed"
  | "prompt_input"

export type RecallEventRole = "user" | "assistant" | "tool" | "system"

export type RecallAttributionConfidence = "definite" | "likely" | "ambiguous" | "unattributed"

export type RecallEvent = {
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
  kind: RecallEventKind
  role?: RecallEventRole | null
  content?: string | null
  content_ref?: string | null
  metadata?: Record<string, unknown>
  candidate_agent_ids?: string[]
  candidate_prompt_ids?: string[]
  candidate_turn_ids?: string[]
  attribution_confidence?: RecallAttributionConfidence | null
  caused_by_event_id?: string | null
}

export type RecallQueryPayload = {
  session_id?: string | null
  agent_id?: string | null
  provider?: string | null
  model?: string | null
  workflow_id?: string | null
  machine_id?: string | null
  repo_root?: string | null
  worktree_path?: string | null
  kind?: RecallEventKind | string | null
  text?: string | null
  after_sequence?: number | null
  before_sequence?: number | null
  limit?: number | null
}

export type RecallEventsPayload = {
  events: RecallEvent[]
  next_sequence: number | null
}

export type SemanticRecallMatch = {
  event: RecallEvent
  score_millis?: number | null
  chunk_index?: number | null
  chunk_text?: string | null
  reason?: string | null
}

export type SemanticSearchRecallMode = "knn" | "agent"

export type SemanticRecallEventsPayload = {
  results: SemanticRecallMatch[]
  next_cursor: string | null
  unavailable_reason: string | null
  answer?: string | null
}

export type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "error" | "status" | "notice" | "turn_toggle"
  text: string
  sourceText?: string
  mergeKey?: string
  source?: "external_provider_observed" | string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
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
  canvas_layout?: WorkflowCanvasLayout | null
  nodes?: WorkflowNodeDefinition[]
  edges?: WorkflowEdgeDefinition[]
  endpoints?: WorkflowEndpointDefinition[]
}

export type WorkflowCanvasPoint = {
  x: number
  y: number
}

export type WorkflowCanvasLayout = {
  version?: number | null
  revision: number
  coordinate_space: string
  nodes?: Record<string, WorkflowCanvasPoint>
  endpoints?: Record<string, WorkflowCanvasPoint>
  edges?: Record<string, { waypoints?: WorkflowCanvasPoint[] }>
}

export type WorkflowDesignWorkflow = {
  id: string
  alias?: string | null
}

export type WorkflowDesignWorkflowPatch = {
  alias?: string | null
  flush_agent_context_before_run?: boolean | null
  run_output_schema_ref?: string | null
  intermediate_output_schema_ref?: string | null
}

export type WorkflowDesignNode = {
  id: string
  agent_id: string
  instructions?: string | null
  can_complete_workflow_run?: boolean | null
  can_emit_intermediate_run_output?: boolean | null
  wait_for_all_inputs?: boolean | null
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowDesignNodePatch = {
  instructions?: string | null
  can_complete_workflow_run?: boolean | null
  can_emit_intermediate_run_output?: boolean | null
  wait_for_all_inputs?: boolean | null
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowDesignEdge = {
  id: string
  from_node_id: string
  to_node_id: string
  source_side?: "top" | "right" | "bottom" | "left" | null
  target_side?: "top" | "right" | "bottom" | "left" | null
  handoff_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowDesignEdgePatch = {
  handoff_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowDesignEndpoint = {
  id: string
  alias?: string | null
  entry_node_id: string
}

export type WorkflowDesignEndpointPatch = {
  alias?: string | null
  entry_node_id?: string | null
}

export type WorkflowDesignOp =
  | { kind: "workflow_create"; workflow: WorkflowDesignWorkflow }
  | { kind: "workflow_update"; workflow_id: string; patch: WorkflowDesignWorkflowPatch }
  | { kind: "workflow_remove"; workflow_id: string }
  | { kind: "node_add"; workflow_id: string; node: WorkflowDesignNode; position?: WorkflowCanvasPoint | null }
  | { kind: "node_update"; workflow_id: string; node_id: string; patch: WorkflowDesignNodePatch }
  | { kind: "node_move"; workflow_id: string; node_id: string; position: WorkflowCanvasPoint }
  | { kind: "node_remove"; workflow_id: string; node_id: string }
  | { kind: "edge_add"; workflow_id: string; edge: WorkflowDesignEdge }
  | { kind: "edge_update"; workflow_id: string; edge_id: string; patch: WorkflowDesignEdgePatch }
  | { kind: "edge_remove"; workflow_id: string; edge_id: string }
  | { kind: "endpoint_add"; workflow_id: string; endpoint: WorkflowDesignEndpoint; position?: WorkflowCanvasPoint | null }
  | { kind: "endpoint_update"; workflow_id: string; endpoint_id: string; patch: WorkflowDesignEndpointPatch }
  | { kind: "endpoint_move"; workflow_id: string; endpoint_id: string; position: WorkflowCanvasPoint }
  | { kind: "endpoint_remove"; workflow_id: string; endpoint_id: string }

export type WorkflowDesignOpForwarded = {
  session_id: string
  origin_client_id: string
  op_id: string
  kernel_sequence: number
  op: WorkflowDesignOp
}

export type WorkflowEndpointDefinition = {
  id: string
  alias: string | null
  entry_node_id: string
}

export type WorkflowPublicationDefinition = {
  id: string
  session_id: string
  workflow_id: string
  endpoint_id: string
  queue_ref?: string | null
  alias?: string | null
  enabled: boolean
  route?: string | null
  methods?: string[]
  transport?: unknown | null
  parser?: unknown | null
  input_schema?: unknown | null
  trace_exposure?: PublicationTraceExposurePolicy | null
  mode?: string | null
  sync_timeout_ms?: number | null
  poll_ms?: number | null
  status?: string | null
  open_url?: string | null
  deployment?: unknown | null
  created_by_user_id: string
  created_at_ms: number
  updated_at_ms: number
}

export type WorkflowPublicationPackageFile = {
  path: string
  content_base64: string
  executable?: boolean
}

export type WorkflowPublicationPackageExportedResponse = {
  WorkflowPublicationPackageExported: {
    publication: WorkflowPublicationDefinition
    package_version: number
    package_digest: string
    package_archive_base64: string
    package_files: WorkflowPublicationPackageFile[]
  }
}

export type PublicationTraceLevel =
  | "output_summary"
  | "assistant_messages"
  | "thinking"
  | "tool_use"

export type PublicationTraceExposurePolicy = {
  nodes?: Record<string, PublicationTraceLevel[]>
}

export type WorkflowPublicationSnapshot = {
  schema_version: number
  captured_at_ms?: number | null
  source_session?: WorkflowPublicationSourceSessionSnapshot | null
  workflow: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition | null
  queues?: WorkflowPromptQueueDefinition[]
  watchdogs?: WorkflowWatchdogDefinition[]
  agents?: AgentInstance[]
}

export type WorkflowPublicationSourceSessionSnapshot = {
  id?: string | null
  alias?: string | null
  workspace_id: string
  worktree_id: string
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

export type WorkflowPromptQueueDefinition = {
  id: string
  workflow_id: string
  alias: string
  priority: number
  enabled: boolean
  created_at_ms: number
  updated_at_ms: number
}

export type WorkflowQueuedPrompt = {
  id: string
  queue_id: string
  workflow_id: string
  endpoint_id: string
  prompt?: string | null
  publication_invocation?: WorkflowPublicationInvocationEnvelope | null
  source: "manual" | "watchdog"
  watchdog_id?: string | null
  status: "queued" | "dispatching" | "running" | "completed" | "cancelled"
  created_at_ms: number
  updated_at_ms: number
  dispatched_at_ms?: number | null
  workflow_run_id?: string | null
}

export type WorkflowPublicationInvocationEnvelope = {
  publication_id: string
  hook_id?: string | null
  invocation_id: string
  transport: string
  endpoint_id: string
  queue_ref?: string | null
  input?: unknown
  artifacts?: unknown[]
  mode?: string | null
  caller?: unknown
}

export type WorkflowNodeDefinition = {
  id: string
  agent_id: string
  owner_user_id?: string
  created_by_user_id?: string
  public_label?: string
  instructions?: string | null
  can_complete_workflow_run?: boolean
  can_emit_intermediate_run_output?: boolean
  wait_for_all_inputs?: boolean
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowEdgeDefinition = {
  id: string
  from_node_id: string
  to_node_id: string
  source_side?: "top" | "right" | "bottom" | "left" | null
  target_side?: "top" | "right" | "bottom" | "left" | null
  handoff_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowMessage = {
  id: string
  source_node_run_id: string | null
  source_node_iteration_index?: number | null
  edge_id?: string | null
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
  iteration_index?: number
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
  thinking_traces?: {
    id: string
    message: string
    timestamp_ms: number
  }[]
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
  publication_invocation?: WorkflowPublicationInvocationEnvelope | null
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
    artifacts?: unknown[]
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
