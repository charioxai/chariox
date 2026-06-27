import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaPreferences } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import type { ThemeRegistry } from "./theme-registry.js"
import type { DirectoryTreeEntry } from "./tree-view.js"


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
  allowed_uses?: string[]
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
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  created_at_ms: number
  last_used_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  hidden?: boolean
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
}

export type WaitingRoomPublicAgentSummary = {
  id: string
  agent_ref: string
  alias?: string | null
  created_at_ms: number
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

export type WaitingRoomRelayStatusView = {
  configured: boolean
  connected: boolean
  relay_url?: string | null
  relay_token_configured: boolean
  daemon_id: string
  machine_id: string
  machine_alias?: string | null
}

export type WaitingRoomRemoteMachineView = {
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

export type WaitingRoomRemoteKernelView = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  kernel_alias?: string | null
  relay_alias?: string | null
  available_providers?: string[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type WaitingRoomTerminalView = {
  terminal_id: string
  terminal_type: "cli" | "web" | "ios" | "android"
  alias?: string | null
  paired_at_ms: number
  revoked: boolean
}

export type ExternalProviderSessionCapabilities = {
  can_read_history?: boolean
}

export type ExternalProviderSessionRecord = {
  external_session_id: string
  provider: string
  provider_session_id: string
  title?: string | null
  title_source?: string | null
  first_prompt_preview?: string | null
  created_at_ms?: number | null
  last_modified_at_ms: number
  worktree_path?: string | null
  account_profile?: string | null
  capabilities?: ExternalProviderSessionCapabilities
}

export type WaitingRoomPublicSnapshot = {
  schema_version: number
  inventory_version: string
  generated_at_ms: number
  sessions: WaitingRoomPublicSessionSummary[]
  relay_status: WaitingRoomRelayStatusView
  remote_machines: WaitingRoomRemoteMachineView[]
  remote_kernels: WaitingRoomRemoteKernelView[]
  terminals?: WaitingRoomTerminalView[]
  external_provider_sessions?: ExternalProviderSessionRecord[]
  external_provider_sessions_has_more?: boolean
  external_provider_sessions_next_cursor?: string | null
  launch_target?: {
    workspace_id: string
    worktree_id: string
    workspace_label?: string | null
    directory?: string | null
    worktree_label?: string | null
  } | null
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

export type WorkspaceLiveSyncTargetStatus = {
  link_id: string
  link_name: string
  user_id: string
  machine_id: string
  kernel_id: string
  repo_root: string
  branch?: string | null
  repo_fingerprint?: string | null
  status: "ready" | "degraded" | "conflict"
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

export type WorkspaceLiveSyncStatus = {
  session_id: string
  mode: "managed" | "tracked" | "unrestricted"
  footer_state: "off" | "managed" | "tracked" | "syncing" | "conflict" | "degraded"
  sync_groups: WorkspaceLiveSyncGroupStatus[]
  targets: WorkspaceLiveSyncTargetStatus[]
  conflicts: Array<{
    conflict_id: string
    link_id: string
    source_agent_id: string
    target_user_id: string
    target_repo_root: string
    path: string
    next_action: string
  }>
  ignore: {
    ignore_file?: string | null
    rules: string[]
    force_excludes: string[]
  }
}

export type AgentPromptState = {
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
}

export type AgentRuntimeActivity = {
  status: "idle" | "working" | "error"
  prompt_status: "none" | "queued" | "running" | "cancelling" | "settling"
  busy: boolean
  unread_idle_output?: boolean
  active_turn?: {
    prompt_id: string
    provider_run_id?: string | null
    prompt_origin?: "arroba" | "external" | string | null
    external_provider?: string | null
    external_provider_session_id?: string | null
    external_provider_turn_id?: string | null
    status: "none" | "queued" | "running" | "cancelling" | "settling"
    phase: "accepted" | "awaiting_first_output" | "streaming" | "settling"
    started_at_ms?: number | null
  } | null
  last_completed_turn?: CompletedGitTurnActionProjection | null
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
    workspace_live_sync?: "off" | "managed" | "tracked"
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

export type SliceDisplayEndpoint = {
  slice_id: string
  kind: "novnc" | "arroba_viewer" | "external"
  url: string
  access: "local" | "tunnel" | "public"
  expires_at_ms?: number | null
  capabilities?: string[]
}

export type SliceRelayEndpoint = {
  url: string
  private?: boolean
}

export type SliceRecord = {
  id: string
  name: string
  owner_kernel_id: string
  owner_machine_id: string
  backend: "local_docker" | "ssh_docker"
  os: string
  status: "stopped" | "starting" | "stopping" | "running" | "unhealthy"
  workspace_mount?: string | null
  workspace_id?: string | null
  worktree_id?: string | null
  session_ids?: string[]
  agent_ids?: string[]
  display_mode?: "headless" | "headed"
  last_operation?: string | null
  last_operation_status?: "accepted" | "in_progress" | "completed" | "failed" | "reconciled" | null
  last_error?: string | null
  last_operation_at_ms?: number | null
  worker_kernel_ref: string
  worker_kernel_id?: string | null
  worker_machine_id?: string | null
  relay_endpoint?: SliceRelayEndpoint | null
  local_docker_ports?: SliceLocalDockerPorts | null
  providers?: string[]
  provider_auth?: Array<{
    provider: string
    state: "unknown" | "not_configured" | "configured" | "authenticated"
    alias?: string | null
    account_id?: string | null
    email?: string | null
    organization_id?: string | null
    organization_name?: string | null
    subscription_type?: string | null
    auth_type?: string | null
    source?: string | null
    checked_at_ms?: number | null
  }>
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

export type AgentInstance = {
  id: string
  agent_ref: string
  session_id: string
  role?: "standard" | "meta" | string
  meta_mode?: {
    task_id?: string | null
    activated_at_ms: number
    baseline_execution_mode_override?: "build" | "plan" | null
    baseline_permission_level_override?: "required" | "yolo" | null
  } | null
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  account_profile?: string | null
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
  prompt_id?: string | null
  source_attachment_id?: string | null
  kind: "provider_output" | "prompt_echo" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status"
  merge_key?: string
  source?: "external_provider_observed" | string | null
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
  observed_at_ms?: number | null
  external_observation?: SessionHistoryExternalObservation | null
  bytes: number[]
}

export type PromptSubmittedPayload = {
  outcome: Record<string, unknown>
  session: RuntimeSession
  agent_activity?: Record<string, AgentRuntimeActivity>
  agent_activity_revision?: number
}

export type QueuedPromptSteeredPayload = {
  prompt: PromptQueueItem
  session: RuntimeSession
  agent_activity?: Record<string, AgentRuntimeActivity>
  agent_activity_revision?: number
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
  external_observation?: SessionHistoryExternalObservation | null
  attachments?: SessionHistoryPromptAttachment[]
  text: string
  timestamp_ms?: number
  source_attachment_id?: string | null
}

export type SessionHistoryExternalObservation = {
  settles_active_prompt: boolean
  passive_telemetry: boolean
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

export type PromptInputHistoryEntry = {
  sequence: number
  timestamp_ms: number
  session_id: string
  source_attachment_id?: string | null
  kind: "prompt" | "command"
  text: string
}

export type PromptInputHistoryPage = {
  entries: PromptInputHistoryEntry[]
}

export type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "error" | "status" | "notice" | "turn_toggle"
  text: string
  promptId?: string | null
  sourceAttachmentId?: string | null
  attachments?: SessionHistoryPromptAttachment[]
  queuedPrompt?: {
    promptId: string
    agentId: string
    status?: "queued" | "steering" | "cancelling"
    steerDisabled?: boolean
  }
  sourceText?: string
  mergeKey?: string
  providerRunId?: string | null
  source?: "external_provider_observed" | string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
  externalObservation?: SessionHistoryExternalObservation | null
  emphasis?: "muted" | "warning" | "error"
  turnId?: number
  hidden?: boolean
  toggleMode?: "expand" | "collapse"
  blobCollapsible?: boolean
  blobCollapsed?: boolean
  blobTitle?: string
  blobSummary?: string
  historyBlobId?: string
  historyBlobAgentId?: string
  historyBlobSourceId?: string
  historyBlobSourceAgentId?: string
  historyBlobLoaded?: boolean
  historyBlobLoading?: boolean
  historyBlobError?: string
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
  schemas?: WorkflowSchemaDefinition[]
  nodes?: WorkflowNodeDefinition[]
  edges?: WorkflowEdgeDefinition[]
  endpoints?: WorkflowEndpointDefinition[]
}

export type WorkflowSchemaDefinition = {
  id: string
  alias?: string | null
  description?: string | null
  schema: unknown
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
  source: "manual" | "watchdog"
  watchdog_id?: string | null
  status: "queued" | "dispatching" | "running" | "completed" | "cancelled"
  created_at_ms: number
  updated_at_ms: number
  dispatched_at_ms?: number | null
  workflow_run_id?: string | null
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
  entries: DirectoryTreeEntry[]
}

export type CliOptions = {
  kernelUrl?: string
  socketPath?: string
  automationSocket?: string
  relayUrl?: string
  relayToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
  detached?: boolean
  sessionId?: string
  createSession?: boolean
  deleteSessionRef?: string
  alias?: string
  clientId: string
  provider?: string
  model: string
  accountProfile: string
  effort: string
  workspace?: string
  worktree?: string
}

export type SessionBinding = {
  session: RuntimeSession
  attachment: RuntimeAttachment
  providerRun: RuntimeProviderRun | null
  createdSession: boolean
  historyEntries: TranscriptEntry[]
  promptHistoryEntries: string[]
  nextHistoryCursor: null
}

export type BootstrapDeferredState = {
  providerCatalog?: Promise<ProviderCatalog>
  providerCommandCatalogs?: Promise<ProviderCommandCatalogs>
  attachedHistory?: Promise<{
    sessionId: string
    visibleAgentId: string | null
    agentEntries: Record<string, TranscriptEntry[]>
    historyEntries: TranscriptEntry[]
    promptHistoryEntries: string[]
    nextHistoryCursor: null
  }>
}

export type BootstrapState = {
  client: LocalIpcClient
  binding: SessionBinding | null
  sessions: RuntimeSession[]
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  options: CliOptions
  preferences: ArrobaPreferences
  themeRegistry?: ThemeRegistry
  deferred?: BootstrapDeferredState
}

export function normalizeAgentPromptState(
  state: Partial<AgentPromptState> | null | undefined,
): AgentPromptState {
  return {
    active_prompt: state?.active_prompt ?? null,
    queued_prompts: Array.isArray(state?.queued_prompts) ? state.queued_prompts : [],
  }
}

export function normalizeRuntimeSession(session: RuntimeSession): RuntimeSession {
  const promptStates = session.prompt_states
    ? Object.fromEntries(
      Object.entries(session.prompt_states).map(([agentId, state]) => [
        agentId,
        normalizeAgentPromptState(state),
      ]),
    )
    : undefined

  const normalized: RuntimeSession = {
    ...session,
    queued_prompts: Array.isArray(session.queued_prompts) ? session.queued_prompts : [],
    active_interactions: Array.isArray(session.active_interactions) ? session.active_interactions : [],
    metaagent_tasks: Array.isArray(session.metaagent_tasks) ? session.metaagent_tasks : [],
  }
  if (promptStates) {
    normalized.prompt_states = promptStates
  }
  return normalized
}

export function normalizeRuntimeSessions(sessions: RuntimeSession[]): RuntimeSession[] {
  return sessions.map((session) => normalizeRuntimeSession(session))
}
