use std::collections::{BTreeMap, HashSet};

use crate::config::UserArchiveHistoryConfig;
use crate::error::DaemonError;
use crate::history::{
    HistoryEventKind, HistoryEventQuery, HistoryEventRole, OperationalHistoryStore,
    SessionHistoryStore,
};
use crate::history_archive::HistoryArchiveClient;
use crate::local::{
    GetPromptInputHistoryRequest, GetSessionHistoryRequest, LocalDaemonResponse,
    PromptInputHistoryEntry, PromptInputHistoryEntryKind, QueryHistoryRequest,
    RecordPromptInputHistoryRequest, SearchHistoryRequest, SemanticHistoryMatch,
    SemanticHistorySearchUtilityInput, SemanticSearchHistoryMode, SemanticSearchHistoryRequest,
};
use crate::runtime::projection::{page_history_entries, SessionHistoryProjectionStore};

pub(crate) async fn execute_session_history_request_from_session(
    history: SessionHistoryStore,
    operational_history: OperationalHistoryStore,
    history_projection: SessionHistoryProjectionStore,
    session: crate::session::RuntimeSession,
    request: GetSessionHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let operational_entries =
            operational_history.load_session_history_entries(session.id(), None)?;
        let entries = if operational_entries.is_empty()
            && !operational_history.has_session_events(session.id())?
            && !operational_history.legacy_fallback_disabled(session.id())?
        {
            history.load(&session)?
        } else {
            operational_entries
        };
        history_projection.update_entries(session.id(), entries.clone());
        let page = page_history_entries(
            entries,
            request.agent_id.as_deref(),
            request.round_count,
            request.max_chars,
            request.before_entry_index,
            request.before_entry_char_offset,
        );
        Ok(LocalDaemonResponse::SessionHistory {
            entries: page.entries,
            next_cursor: page.next_cursor,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history",
        message: error.to_string(),
    })?
}

pub(crate) async fn execute_prompt_input_history_request(
    history: OperationalHistoryStore,
    request: GetPromptInputHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let session_id = request.session_id.clone();
    let limit = request.limit.unwrap_or(5000).clamp(1, 5000);
    let after_sequence = request.after_sequence;
    tokio::task::spawn_blocking(move || {
        let mut events = prompt_input_history_events_for_kind(
            &history,
            &session_id,
            "user_prompt",
            after_sequence,
            limit,
        )?;
        events.extend(prompt_input_history_events_for_kind(
            &history,
            &session_id,
            "prompt_input",
            after_sequence,
            limit,
        )?);
        events.sort_by_key(|event| event.sequence);
        events.truncate(limit);
        Ok(LocalDaemonResponse::PromptInputHistory {
            entries: events
                .into_iter()
                .filter_map(prompt_input_history_entry_from_event)
                .collect(),
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load prompt input history",
        message: error.to_string(),
    })?
}

pub(crate) async fn execute_record_prompt_input_history_request(
    history: OperationalHistoryStore,
    request: RecordPromptInputHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.text.trim().is_empty() {
        return Ok(LocalDaemonResponse::PromptInputHistoryRecorded {
            entry: PromptInputHistoryEntry {
                sequence: 0,
                timestamp_ms: 0,
                session_id: request.session_id,
                source_attachment_id: request.attachment_id,
                kind: request.kind,
                text: String::new(),
            },
        });
    }
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "input_kind".to_string(),
        serde_json::Value::String(
            match request.kind {
                PromptInputHistoryEntryKind::Prompt => "prompt",
                PromptInputHistoryEntryKind::Command => "command",
            }
            .to_string(),
        ),
    );
    if let Some(attachment_id) = request.attachment_id.clone() {
        metadata.insert(
            "source_attachment_id".to_string(),
            serde_json::Value::String(attachment_id),
        );
    }
    let event = history.append_operational_event(
        HistoryEventKind::PromptInput,
        Some(HistoryEventRole::User),
        Some(request.text),
        metadata,
        crate::history::HistoryEventTurnContext {
            session_id: Some(request.session_id),
            ..Default::default()
        },
    )?;
    let entry = prompt_input_history_entry_from_event(event).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "record prompt input history",
            message: "recorded event could not be converted".to_string(),
        }
    })?;
    Ok(LocalDaemonResponse::PromptInputHistoryRecorded { entry })
}

pub(crate) async fn execute_query_history_request(
    history: OperationalHistoryStore,
    archive_config: UserArchiveHistoryConfig,
    query: HistoryEventQuery,
) -> Result<LocalDaemonResponse, DaemonError> {
    let requested_limit = query.limit.unwrap_or(100).clamp(1, 500);
    tokio::task::spawn_blocking(move || {
        let mut events = history.query_events(query.clone())?;
        let archive_client = HistoryArchiveClient::from_config(&archive_config)?;
        let archive_capabilities = archive_client.capabilities().ok();
        if archive_capabilities
            .as_ref()
            .map(|capabilities| capabilities.search)
            .unwrap_or(false)
        {
            let archive_response = archive_client.search_events(query.clone())?;
            merge_history_events(&mut events, archive_response.events);
        }
        events.sort_by(|left, right| {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        events.truncate(requested_limit);
        let next_sequence = if query.before_sequence.is_none() && events.len() == requested_limit {
            events.last().map(|event| event.sequence)
        } else {
            None
        };
        Ok(LocalDaemonResponse::HistoryEvents {
            events,
            next_sequence,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "query history",
        message: error.to_string(),
    })?
}

pub(crate) async fn knn_semantic_history_search(
    archive_config: UserArchiveHistoryConfig,
    request: SemanticSearchHistoryRequest,
    requested_limit: usize,
) -> Result<(Vec<SemanticHistoryMatch>, Option<String>, Option<String>), DaemonError> {
    let response = tokio::task::spawn_blocking(move || {
        let archive_client = HistoryArchiveClient::from_config(&archive_config)?;
        let archive_capabilities = archive_client.capabilities().ok();
        if !archive_capabilities
            .as_ref()
            .map(|capabilities| capabilities.semantic_search || capabilities.vector_search)
            .unwrap_or(false)
        {
            return Ok((
                Vec::new(),
                None,
                Some("semantic history search is not configured for this kernel".to_string()),
            ));
        }
        let cursor = request.cursor.clone();
        let mut query = history_query_from_semantic_search_request(request);
        query.limit = Some(requested_limit);
        let response = archive_client.semantic_search_events(query, cursor)?;
        Ok((response.results, response.next_cursor, None))
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "semantic search history",
        message: error.to_string(),
    })??;
    Ok(response)
}

pub(crate) fn history_query_from_request(request: QueryHistoryRequest) -> HistoryEventQuery {
    HistoryEventQuery {
        session_id: request.session_id,
        agent_id: request.agent_id,
        provider: request.provider,
        model: request.model,
        workflow_id: request.workflow_id,
        machine_id: request.machine_id,
        repo_root: request.repo_root,
        worktree_path: request.worktree_path,
        kind: request.kind,
        text: request.text,
        after_sequence: request.after_sequence,
        before_sequence: request.before_sequence,
        limit: request.limit,
    }
}

pub(crate) fn history_query_from_search_request(
    request: SearchHistoryRequest,
) -> HistoryEventQuery {
    HistoryEventQuery {
        session_id: request.session_id,
        agent_id: request.agent_id,
        provider: request.provider,
        model: request.model,
        workflow_id: request.workflow_id,
        machine_id: request.machine_id,
        repo_root: request.repo_root,
        worktree_path: request.worktree_path,
        kind: request.kind,
        text: Some(request.query),
        after_sequence: request.after_sequence,
        before_sequence: None,
        limit: request.limit,
    }
}

pub(crate) fn semantic_utility_input_from_search_request(
    request: SemanticSearchHistoryRequest,
) -> SemanticHistorySearchUtilityInput {
    SemanticHistorySearchUtilityInput {
        query: request.query,
        session_id: request.session_id,
        agent_id: request.agent_id,
        provider: request.provider,
        model: request.model,
        workflow_id: request.workflow_id,
        machine_id: request.machine_id,
        repo_root: request.repo_root,
        worktree_path: request.worktree_path,
        kind: request.kind,
        limit: request.limit,
    }
}

pub(crate) fn semantic_search_request_from_utility_input(
    input: &SemanticHistorySearchUtilityInput,
) -> SemanticSearchHistoryRequest {
    SemanticSearchHistoryRequest {
        query: input.query.clone(),
        mode: Some(SemanticSearchHistoryMode::Knn),
        session_id: input.session_id.clone(),
        agent_id: input.agent_id.clone(),
        provider: input.provider.clone(),
        model: input.model.clone(),
        workflow_id: input.workflow_id.clone(),
        machine_id: input.machine_id.clone(),
        repo_root: input.repo_root.clone(),
        worktree_path: input.worktree_path.clone(),
        kind: input.kind.clone(),
        cursor: None,
        limit: input.limit,
    }
}

fn prompt_input_history_entry_from_event(
    event: crate::history::HistoryEvent,
) -> Option<PromptInputHistoryEntry> {
    let session_id = event.session_id.clone()?;
    let kind = match event.kind {
        HistoryEventKind::UserPrompt => PromptInputHistoryEntryKind::Prompt,
        HistoryEventKind::PromptInput => match event
            .metadata
            .get("input_kind")
            .and_then(|value| value.as_str())
        {
            Some("command") => PromptInputHistoryEntryKind::Command,
            _ => PromptInputHistoryEntryKind::Prompt,
        },
        _ => return None,
    };
    Some(PromptInputHistoryEntry {
        sequence: event.sequence,
        timestamp_ms: event.timestamp_ms,
        session_id,
        source_attachment_id: event
            .metadata
            .get("source_attachment_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        kind,
        text: event.content.unwrap_or_default(),
    })
}

fn prompt_input_history_events_for_kind(
    history: &OperationalHistoryStore,
    session_id: &str,
    kind: &str,
    after_sequence: Option<u64>,
    limit: usize,
) -> Result<Vec<crate::history::HistoryEvent>, DaemonError> {
    let mut events = Vec::new();
    let mut cursor = after_sequence;
    while events.len() < limit {
        let batch = history.query_events(HistoryEventQuery {
            session_id: Some(session_id.to_string()),
            kind: Some(kind.to_string()),
            after_sequence: cursor,
            limit: Some((limit - events.len()).min(500)),
            ..HistoryEventQuery::default()
        })?;
        let Some(last_sequence) = batch.last().map(|event| event.sequence) else {
            break;
        };
        let batch_len = batch.len();
        events.extend(batch);
        cursor = Some(last_sequence);
        if batch_len < 500 {
            break;
        }
    }
    Ok(events)
}

fn history_query_from_semantic_search_request(
    request: SemanticSearchHistoryRequest,
) -> HistoryEventQuery {
    HistoryEventQuery {
        session_id: request.session_id,
        agent_id: request.agent_id,
        provider: request.provider,
        model: request.model,
        workflow_id: request.workflow_id,
        machine_id: request.machine_id,
        repo_root: request.repo_root,
        worktree_path: request.worktree_path,
        kind: request.kind,
        text: Some(request.query),
        after_sequence: None,
        before_sequence: None,
        limit: request.limit,
    }
}

fn merge_history_events(
    events: &mut Vec<crate::history::HistoryEvent>,
    archive_events: Vec<crate::history::HistoryEvent>,
) {
    let mut seen = events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<HashSet<_>>();
    for event in archive_events {
        if seen.insert(event.event_id.clone()) {
            events.push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn history_event(event_id: &str, sequence: u64) -> crate::history::HistoryEvent {
        crate::history::HistoryEvent {
            event_id: event_id.to_string(),
            sequence,
            timestamp_ms: sequence,
            workspace_id: None,
            session_id: None,
            agent_id: None,
            agent_alias: None,
            provider: None,
            model: None,
            turn_id: None,
            prompt_id: None,
            provider_run_id: None,
            provider_session_id: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_node_id: None,
            machine_id: None,
            repo_root: None,
            worktree_path: None,
            kind: HistoryEventKind::Notice,
            role: None,
            content: None,
            content_ref: None,
            metadata: BTreeMap::new(),
            candidate_agent_ids: Vec::new(),
            candidate_prompt_ids: Vec::new(),
            candidate_turn_ids: Vec::new(),
            attribution_confidence: None,
            caused_by_event_id: None,
        }
    }

    #[test]
    fn history_query_from_search_request_maps_text_and_pagination() {
        let query = history_query_from_search_request(SearchHistoryRequest {
            query: "deploy".to_string(),
            session_id: Some("session-1".to_string()),
            agent_id: None,
            provider: None,
            model: None,
            workflow_id: None,
            machine_id: None,
            repo_root: None,
            worktree_path: None,
            kind: None,
            after_sequence: Some(42),
            limit: Some(8),
        });

        assert_eq!(query.text.as_deref(), Some("deploy"));
        assert_eq!(query.session_id.as_deref(), Some("session-1"));
        assert_eq!(query.after_sequence, Some(42));
        assert_eq!(query.before_sequence, None);
        assert_eq!(query.limit, Some(8));
    }

    #[test]
    fn semantic_utility_request_forces_knn_mode_and_preserves_filters() {
        let request =
            semantic_search_request_from_utility_input(&SemanticHistorySearchUtilityInput {
                query: "why did tests fail?".to_string(),
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                provider: Some("codex".to_string()),
                model: None,
                workflow_id: None,
                machine_id: None,
                repo_root: None,
                worktree_path: Some("/repo".to_string()),
                kind: Some("terminal_output".to_string()),
                limit: Some(12),
            });

        assert_eq!(request.mode, Some(SemanticSearchHistoryMode::Knn));
        assert_eq!(request.cursor, None);
        assert_eq!(request.session_id.as_deref(), Some("session-1"));
        assert_eq!(request.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(request.worktree_path.as_deref(), Some("/repo"));
        assert_eq!(request.kind.as_deref(), Some("terminal_output"));
        assert_eq!(request.limit, Some(12));
    }

    #[test]
    fn merge_history_events_deduplicates_by_event_id() {
        let mut events = vec![history_event("event-1", 1)];
        merge_history_events(
            &mut events,
            vec![history_event("event-1", 2), history_event("event-2", 3)],
        );

        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["event-1", "event-2"]
        );
    }
}
