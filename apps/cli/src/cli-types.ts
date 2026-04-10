import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaPreferences } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import type { DirectoryTreeEntry } from "./tree-view.js"

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

export type AgentInstance = {
  id: string
  agent_ref: string
  session_id: string
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  worktree_id: string | null
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
  entries: DirectoryTreeEntry[]
}

export type CliOptions = {
  kernelUrl?: string
  socketPath?: string
  relayUrl?: string
  relayToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
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
  nextHistoryCursor: SessionHistoryCursor | null
}

export type BootstrapState = {
  client: LocalIpcClient
  binding: SessionBinding | null
  sessions: RuntimeSession[]
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  options: CliOptions
  preferences: ArrobaPreferences
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

  return {
    ...session,
    queued_prompts: Array.isArray(session.queued_prompts) ? session.queued_prompts : [],
    prompt_states: promptStates,
  }
}

export function normalizeRuntimeSessions(sessions: RuntimeSession[]): RuntimeSession[] {
  return sessions.map((session) => normalizeRuntimeSession(session))
}
