import type { ExtensionGrant } from "./kernel-types-extensions.js"
import type { ExternalProviderImportMetadata, RuntimeSession } from "./kernel-types-session.js"
import type { PromptAttachmentPart, RuntimeProviderRun } from "./kernel-types-provider.js"

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
  prompt_status: "none" | "queued" | "dispatching" | "running" | "cancelling" | "settling"
  busy: boolean
  active_prompt_count?: number
  queued_prompt_count?: number
  unread_idle_output: boolean
  queued_prompt_controls?: Record<string, AgentQueuedPromptControl>
  active_turn?: AgentActiveTurn | null
  last_completed_turn?: CompletedGitTurnActionProjection | null
}

export type AgentQueuedPromptControl = {
  prompt_id: string
  status: string
  can_steer: boolean
  can_cancel: boolean
  steer_disabled_reason?: string | null
  cancel_disabled_reason?: string | null
}

export type AgentActiveTurn = {
  prompt_id: string
  provider_run_id?: string | null
  source_attachment_id?: string | null
  prompt_origin?: "arroba" | "external" | string | null
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
  status: "none" | "queued" | "running" | "cancelling" | "settling"
  phase: "accepted" | "awaiting_first_output" | "streaming" | "settling"
  started_at_ms?: number | null
}

export type CompletedGitTurnActionProjection = {
  turn_id: string
  prompt_id: string
  provider_run_id: string
  agent_id: string
  prompt_origin?: "arroba" | "external" | string | null
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
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
  workflow?: {
    max_queues_per_workflow?: number
    session_default_max_agents?: number
    code?: {
      max_concurrent?: number
      max_nodes?: number
      max_agents?: number
      max_edges?: number
      max_queues?: number
      max_watchdogs?: number
      max_schema_bytes?: number
      max_generated_prompt_bytes?: number
      script_timeout_ms?: number
      script_memory_bytes?: number
    }
  }
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
  pending_prompt_id?: string | null
  source_attachment_id: string
  target_agent_id?: string | null
  prompt: string
  attachments?: PromptAttachmentPart[]
  created_at_ms?: number
  updated_at_ms?: number
  status: string
  prompt_origin?: "arroba" | "external" | string
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
}

export type RuntimeAttachment = {
  id: string
  session_id: string
}
