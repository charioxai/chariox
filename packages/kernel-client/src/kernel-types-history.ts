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
  prompt_origin?: "chariox" | "external" | string | null
  prompt_source?: "human" | "agent_terminal" | "provider_external" | string | null
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
  prompt_origin?: "chariox" | "external" | string | null
  prompt_source?: "human" | "agent_terminal" | "provider_external" | string | null
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
  started_at_ms: number
  lifecycle: "open" | "completed" | "cancelled" | string
  completed_at_ms: number | null
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
