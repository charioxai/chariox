import type { ExtensionGrant } from "./kernel-types-extensions.js"
import type {
  AgentInstance,
  AgentPromptState,
  AgentRuntimeActivity,
  PromptQueueItem,
  RuntimeInteraction,
  SessionInvite,
  SessionMember,
  SessionConfigState,
} from "./kernel-types-runtime.js"
import type {
  WorkflowConsole,
  WorkflowDefinition,
  WorkflowPromptQueueDefinition,
  WorkflowPublicationDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
  WorkflowScheduleDefinition,
  WorkflowWatchdogDefinition,
} from "./kernel-types-workflow.js"

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
  agent_activity_revision?: number
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
  workflow_schedules?: WorkflowScheduleDefinition[]
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
  arroba_owned_observed_prompt_turn_ids?: string[]
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
