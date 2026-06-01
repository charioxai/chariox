import type {
  RecallQueryPayload,
  SemanticSearchRecallMode,
} from "./kernel-types.js"

export function getSessionHistoryOutlineRequest(
  sessionId: string,
  agentIds?: readonly string[] | null,
  latestPromptCount?: number | null,
) {
  return {
    GetSessionHistoryOutline: {
      session_id: sessionId,
      agent_ids: agentIds ? [...agentIds] : null,
      latest_prompt_count: latestPromptCount ?? null,
    },
  }
}

export function getSessionHistoryBlobContentRequest(
  sessionId: string,
  agentId: string,
  blobId: string,
) {
  return {
    GetSessionHistoryBlobContent: {
      session_id: sessionId,
      agent_id: agentId,
      blob_id: blobId,
    },
  }
}

export function getPromptInputHistoryRequest(
  sessionId: string,
  afterSequence?: number | null,
  limit?: number | null,
) {
  return {
    GetPromptInputHistory: {
      session_id: sessionId,
      after_sequence: afterSequence ?? null,
      limit: limit ?? null,
    },
  }
}

export function recordPromptInputHistoryRequest(
  sessionId: string,
  attachmentId: string | null,
  kind: "prompt" | "command",
  text: string,
) {
  return {
    RecordPromptInputHistory: {
      session_id: sessionId,
      attachment_id: attachmentId,
      kind,
      text,
    },
  }
}

export function queryRecallRequest(query: RecallQueryPayload) {
  return {
    QueryRecall: {
      session_id: query.session_id ?? null,
      agent_id: query.agent_id ?? null,
      provider: query.provider ?? null,
      model: query.model ?? null,
      workflow_id: query.workflow_id ?? null,
      machine_id: query.machine_id ?? null,
      repo_root: query.repo_root ?? null,
      worktree_path: query.worktree_path ?? null,
      kind: query.kind ?? null,
      text: query.text ?? null,
      after_sequence: query.after_sequence ?? null,
      before_sequence: query.before_sequence ?? null,
      limit: query.limit ?? null,
    },
  }
}

export function searchRecallRequest(query: string, filters: Omit<RecallQueryPayload, "text"> = {}) {
  return {
    SearchRecall: {
      query,
      session_id: filters.session_id ?? null,
      agent_id: filters.agent_id ?? null,
      provider: filters.provider ?? null,
      model: filters.model ?? null,
      workflow_id: filters.workflow_id ?? null,
      machine_id: filters.machine_id ?? null,
      repo_root: filters.repo_root ?? null,
      worktree_path: filters.worktree_path ?? null,
      kind: filters.kind ?? null,
      after_sequence: filters.after_sequence ?? null,
      limit: filters.limit ?? null,
    },
  }
}

export function semanticSearchRecallRequest(
  query: string,
  filters: Omit<RecallQueryPayload, "text" | "after_sequence" | "before_sequence"> & {
    mode?: SemanticSearchRecallMode | null
    cursor?: string | null
  } = {},
) {
  return {
    SemanticSearchRecall: {
      query,
      mode: filters.mode ?? null,
      session_id: filters.session_id ?? null,
      agent_id: filters.agent_id ?? null,
      provider: filters.provider ?? null,
      model: filters.model ?? null,
      workflow_id: filters.workflow_id ?? null,
      machine_id: filters.machine_id ?? null,
      repo_root: filters.repo_root ?? null,
      worktree_path: filters.worktree_path ?? null,
      kind: filters.kind ?? null,
      cursor: filters.cursor ?? null,
      limit: filters.limit ?? null,
    },
  }
}
