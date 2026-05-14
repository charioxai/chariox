//! Operational history query and archive merge support.

use std::collections::{BTreeMap, HashSet};

use crate::config::UserArchiveHistoryConfig;
use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, HistoryEventQuery, OperationalHistoryStore};
use crate::history_archive::HistoryArchiveClient;
use crate::local::{LocalDaemonResponse, QueryHistoryRequest, SearchHistoryRequest};

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

fn merge_history_events(events: &mut Vec<HistoryEvent>, archive_events: Vec<HistoryEvent>) {
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

    fn history_event(event_id: &str, sequence: u64) -> HistoryEvent {
        HistoryEvent {
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
