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
  owner_user_id?: string
  members?: SessionMember[]
  invites?: SessionInvite[]
  created_at_ms: number
  last_used_at_ms?: number | null
  status: string
  agent_defaults?: SessionAgentDefaults
  active_provider_run_id: string | null
  attachment_ids: string[]
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
  prompt_states?: Record<string, AgentPromptState>
  agent_activity?: Record<string, AgentRuntimeActivity>
  active_interactions?: RuntimeInteraction[]
  focused_agent_id: string | null
  max_agents: number
  agents: AgentInstance[]
  config_state: SessionConfigState
  workflow_launch_policy?: "reject" | "queue" | null
  workflows?: WorkflowDefinition[]
  workflow_publications?: WorkflowPublicationDefinition[]
  workflow_runs?: WorkflowRun[]
  queued_workflow_launches?: QueuedWorkflowLaunch[]
  workflow_watchdogs?: WorkflowWatchdogDefinition[]
  workflow_consoles?: WorkflowConsole[]
  workspace_links?: WorkspaceLinkDefinition[]
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

export type SessionMember = {
  user_id: string
  joined_at_ms: number
  invited_by_user_id?: string | null
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
}

export type AgentPromptState = {
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
}

export type AgentRuntimeActivity = {
  status: "idle" | "working" | "error"
  prompt_status: "none" | "queued" | "running" | "cancelling" | "settling"
  busy: boolean
  active_turn?: AgentActiveTurn | null
}

export type AgentActiveTurn = {
  prompt_id: string
  provider_run_id?: string | null
  status: "none" | "queued" | "running" | "cancelling" | "settling"
}

export type RuntimeInteraction = {
  id: string
  agent_id: string
  kind: "choice" | "permission"
  level: "info" | "warning" | "critical"
  title?: string | null
  message: string
  choices: RuntimeInteractionChoice[]
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
    managed_io?: "required" | "unrestricted"
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
    backend?: "os_keychain"
    service?: string
  }
  credentials?: Array<{
    id: string
    description?: string
    source?: Record<string, unknown>
    allowed_hosts?: string[]
    allowed_uses?: string[]
    injection?: Record<string, unknown>
  }>
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
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  primary_provider?: string | null
  primary_model?: string | null
  primary_effort?: string | null
  execution_mode_override?: "build" | "plan" | null
  permission_level_override?: "required" | "yolo" | null
  worktree_id: string | null
  remote_execution?: {
    worker_kernel_id: string
    worker_machine_id: string
    execution_lease_id: string
    leased_agent_id: string
  } | null
  mcp_grants?: string[]
  skill_grants?: string[]
  substitutes?: AgentSubstituteProfile[]
  active_substitute_index?: number | null
  last_substitution?: AgentSubstitutionRecord | null
  substitution_timeout_ms?: number | null
  state: "Idle" | "Working" | "Focused" | "Error"
  is_processing: boolean
  grid_row: number
  grid_col: number
  grid_row_span: number
  grid_col_span: number
  created_at_ms: number
  last_activity_at_ms: number
}

export type AgentSubstituteProfile = {
  provider: string
  model: string
  variant?: string | null
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
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type WaitingRoomInventorySnapshot = {
  inventory_version: string
  sessions: WaitingRoomPublicSessionSummary[]
  relay_status: RelayStatus
  remote_machines: RemoteMachineRecord[]
  remote_kernels: RelayKernelPresence[]
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
  started_at_ms?: number
  last_activity_at_ms?: number
  control_capabilities?: {
    operation: string
    mode: string
  }[]
}

export const LOCAL_DAEMON_PROTOCOL_VERSION = 12

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
  | "prompt_input"

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

export type WorkflowPublicationDefinition = {
  id: string
  session_id: string
  workflow_id: string
  endpoint_id: string
  alias?: string | null
  enabled: boolean
  route?: string | null
  methods?: string[]
  transport?: unknown | null
  auth?: unknown | null
  parser?: unknown | null
  input_schema?: unknown | null
  mode?: string | null
  pairing_codes?: WorkflowPublicationPairingCode[]
  trusted_senders?: WorkflowPublicationTrustedSender[]
  created_by_user_id: string
  created_at_ms: number
  updated_at_ms: number
}

export type WorkflowPublicationPairingCode = {
  code_id: string
  publication_id: string
  pair_code_hash: string
  created_by_user_id: string
  created_at_ms: number
  expires_at_ms?: number | null
  max_uses?: number | null
  used_count: number
  revoked_at_ms?: number | null
}

export type WorkflowPublicationPairingCodeRecord = {
  code: WorkflowPublicationPairingCode
  pair_code: string
}

export type WorkflowPublicationTrustedSender = {
  sender_id: string
  publication_id: string
  display_name?: string | null
  credential_hash: string
  allowed_transports?: string[]
  created_at_ms: number
  last_used_at_ms?: number | null
  expires_at_ms?: number | null
  revoked_at_ms?: number | null
}

export type WorkflowPublicationSenderCredential = {
  sender: WorkflowPublicationTrustedSender
  credential: string
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
