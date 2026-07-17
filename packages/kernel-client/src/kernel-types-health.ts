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
  worker_extension_agents?: number
  worker_extension_grants?: number
  worker_manifest_missing_agents?: number
  worker_synced_agents?: number
  worker_syncing_agents?: number
  worker_pending_agents?: number
  worker_failed_agents?: number
  worker_stale_agents?: number
  worker_pending_revoke_agents?: number
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
  source?: "home" | "worker"
  home_proxy_grants: string[]
  worker_grants?: string[]
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
