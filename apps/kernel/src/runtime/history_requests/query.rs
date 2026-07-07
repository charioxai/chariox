//! Operational recall query and archive merge support.

use std::collections::HashSet;

use crate::config::UserArchiveHistoryConfig;
use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventQuery, OperationalHistoryStore};
use crate::history_archive::{HistoryArchiveClient, HistoryArchiveSearchResponse};
use crate::local::{LocalDaemonResponse, QueryRecallRequest, SearchRecallRequest};

pub(crate) async fn execute_query_recall_request(
    history: OperationalHistoryStore,
    archive_config: UserArchiveHistoryConfig,
    query: HistoryEventQuery,
) -> Result<LocalDaemonResponse, DaemonError> {
    let requested_limit = query.limit.unwrap_or(100).clamp(1, 500);
    tokio::task::spawn_blocking(move || {
        let mut events = query_projected_history_events(&history, query.clone(), requested_limit)?;
        let archive_client = HistoryArchiveClient::from_config(&archive_config)?;
        let archive_capabilities = archive_client.capabilities().ok();
        if archive_capabilities
            .as_ref()
            .map(|capabilities| capabilities.search)
            .unwrap_or(false)
        {
            let archive_events =
                query_projected_archive_history_events(query.clone(), requested_limit, |query| {
                    archive_client.search_events(query)
                })?;
            merge_history_events(&mut events, archive_events);
        }
        events.sort_by(|left, right| {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        events.retain(history_event_projects_as_recall_result);
        events.truncate(requested_limit);
        let next_sequence = if query.before_sequence.is_none() && events.len() == requested_limit {
            events.last().map(|event| event.sequence)
        } else {
            None
        };
        Ok(LocalDaemonResponse::RecallEvents {
            events,
            next_sequence,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "query recall",
        message: error.to_string(),
    })?
}

pub(crate) fn recall_query_from_request(request: QueryRecallRequest) -> HistoryEventQuery {
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

pub(crate) fn recall_query_from_search_request(request: SearchRecallRequest) -> HistoryEventQuery {
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

fn query_projected_history_events(
    history: &OperationalHistoryStore,
    query: HistoryEventQuery,
    requested_limit: usize,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    let raw_limit = requested_limit.clamp(1, 500);
    let reverse_before_page = query.before_sequence.is_some() && query.after_sequence.is_none();
    let mut page_query = query;
    page_query.limit = Some(raw_limit);
    let mut projected_events = Vec::new();

    while projected_events.len() < requested_limit {
        let raw_events = history.query_events(page_query.clone())?;
        let Some(first_sequence) = raw_events.first().map(|event| event.sequence) else {
            break;
        };
        let last_sequence = raw_events
            .last()
            .map(|event| event.sequence)
            .unwrap_or(first_sequence);
        let raw_len = raw_events.len();

        projected_events.extend(
            raw_events
                .into_iter()
                .filter(history_event_projects_as_recall_result),
        );

        if raw_len < raw_limit {
            break;
        }

        if reverse_before_page {
            page_query.before_sequence = Some(first_sequence);
        } else {
            page_query.after_sequence = Some(last_sequence);
        }
    }

    projected_events.sort_by(|left, right| {
        left.sequence
            .cmp(&right.sequence)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    projected_events.truncate(requested_limit);
    Ok(projected_events)
}

fn query_projected_archive_history_events<F>(
    query: HistoryEventQuery,
    requested_limit: usize,
    mut fetch: F,
) -> Result<Vec<HistoryEvent>, DaemonError>
where
    F: FnMut(HistoryEventQuery) -> Result<HistoryArchiveSearchResponse, DaemonError>,
{
    let requested_limit = requested_limit.clamp(1, 500);
    let raw_limit = requested_limit;
    let can_page_forward = query.before_sequence.is_none();
    let mut page_query = query;
    page_query.limit = Some(raw_limit);
    let mut projected_events = Vec::new();

    while projected_events.len() < requested_limit {
        let previous_after_sequence = page_query.after_sequence;
        let response = fetch(page_query.clone())?;
        let raw_len = response.events.len();
        projected_events.extend(
            response
                .events
                .into_iter()
                .filter(history_event_projects_as_recall_result),
        );
        if projected_events.len() >= requested_limit || raw_len == 0 || !can_page_forward {
            break;
        }
        let Some(next_sequence) = response.next_sequence else {
            break;
        };
        if previous_after_sequence.is_some_and(|previous| next_sequence <= previous) {
            break;
        }
        page_query.after_sequence = Some(next_sequence);
    }

    projected_events.sort_by(|left, right| {
        left.sequence
            .cmp(&right.sequence)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    projected_events.truncate(requested_limit);
    Ok(projected_events)
}

fn history_event_projects_as_recall_result(event: &HistoryEvent) -> bool {
    event
        .to_session_history_entry()
        .is_none_or(|entry| !entry.is_external_provider_observed_state_signal())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::history::{
        HistoryEventKind, HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryEntryKind,
    };

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

    fn temp_history_store(name: &str) -> (OperationalHistoryStore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "arroba-recall-{name}-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        (
            OperationalHistoryStore::open(path.clone()).expect("operational history should open"),
            path,
        )
    }

    fn observed_output(text: &str, turn_id: &str, observed_at_ms: u64) -> SessionHistoryEntry {
        SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            text,
            "codex",
            "thread-1",
            Some(turn_id.to_string()),
            Some(observed_at_ms),
        )
    }

    fn observed_state_signal(turn_id: &str, observed_at_ms: u64) -> SessionHistoryEntry {
        SessionHistoryEntry::external_provider_observed_state_signal(
            "session-1",
            Some("run-1"),
            "agent-1",
            "codex",
            "thread-1",
            crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
            &format!("external:codex:thread-1:{turn_id}"),
            turn_id.to_string(),
            Some(observed_at_ms),
        )
    }

    fn append_transcript_event(
        store: &OperationalHistoryStore,
        sequence: u64,
        entry: &SessionHistoryEntry,
    ) {
        store
            .append(&HistoryEvent::transcript(
                sequence,
                entry,
                HistoryEventTurnContext {
                    session_id: Some("session-1".to_string()),
                    agent_id: Some("agent-1".to_string()),
                    ..HistoryEventTurnContext::default()
                },
            ))
            .expect("history event should append");
    }

    #[test]
    fn recall_query_from_search_request_maps_text_and_pagination() {
        let query = recall_query_from_search_request(SearchRecallRequest {
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

    #[test]
    fn recall_results_hide_external_observer_state_signals() {
        let visible = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            "visible output",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_000),
        );
        let hidden = SessionHistoryEntry::external_provider_observed_state_signal(
            "session-1",
            Some("run-1"),
            "agent-1",
            "codex",
            "thread-1",
            crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
            "external:codex:thread-1:turn-1",
            "turn-1".to_string(),
            Some(2_100),
        );
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let visible_event = HistoryEvent::transcript(1, &visible, context.clone());
        let hidden_event = HistoryEvent::transcript(2, &hidden, context);

        assert!(history_event_projects_as_recall_result(&visible_event));
        assert!(!history_event_projects_as_recall_result(&hidden_event));
    }

    #[test]
    fn recall_query_pages_past_hidden_state_signals() {
        let (store, path) = temp_history_store("forward-hidden");
        append_transcript_event(
            &store,
            1,
            &observed_output("first visible", "turn-1", 1_000),
        );
        append_transcript_event(&store, 2, &observed_state_signal("turn-1", 1_100));
        append_transcript_event(
            &store,
            3,
            &observed_output("second visible", "turn-2", 1_200),
        );

        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        let response = runtime
            .block_on(execute_query_recall_request(
                store.clone(),
                UserArchiveHistoryConfig::default(),
                HistoryEventQuery {
                    session_id: Some("session-1".to_string()),
                    limit: Some(2),
                    ..HistoryEventQuery::default()
                },
            ))
            .expect("recall query should load");
        let LocalDaemonResponse::RecallEvents {
            events,
            next_sequence,
        } = response
        else {
            panic!("expected recall events response");
        };

        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.content.as_deref())
                .collect::<Vec<_>>(),
            vec!["first visible", "second visible"]
        );
        assert_eq!(next_sequence, Some(3));
        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn recall_query_loads_older_page_past_hidden_state_signals() {
        let (store, path) = temp_history_store("before-hidden");
        append_transcript_event(
            &store,
            1,
            &observed_output("older visible", "turn-1", 1_000),
        );
        append_transcript_event(&store, 2, &observed_state_signal("turn-1", 1_100));
        append_transcript_event(
            &store,
            3,
            &observed_output("newer visible", "turn-2", 1_200),
        );

        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        let response = runtime
            .block_on(execute_query_recall_request(
                store.clone(),
                UserArchiveHistoryConfig::default(),
                HistoryEventQuery {
                    session_id: Some("session-1".to_string()),
                    before_sequence: Some(4),
                    limit: Some(2),
                    ..HistoryEventQuery::default()
                },
            ))
            .expect("recall query should load");
        let LocalDaemonResponse::RecallEvents {
            events,
            next_sequence,
        } = response
        else {
            panic!("expected recall events response");
        };

        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.content.as_deref())
                .collect::<Vec<_>>(),
            vec!["older visible", "newer visible"]
        );
        assert_eq!(next_sequence, None);
        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn archive_recall_pages_past_hidden_state_signals() {
        let hidden = observed_state_signal("turn-1", 1_100);
        let visible = observed_output("archived visible", "turn-1", 1_200);
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let mut calls = Vec::new();

        let events = query_projected_archive_history_events(
            HistoryEventQuery {
                session_id: Some("session-1".to_string()),
                limit: Some(1),
                ..HistoryEventQuery::default()
            },
            1,
            |query| {
                calls.push((query.after_sequence, query.limit));
                let response = match query.after_sequence {
                    None => HistoryArchiveSearchResponse {
                        events: vec![HistoryEvent::transcript(1, &hidden, context.clone())],
                        next_sequence: Some(1),
                    },
                    Some(1) => HistoryArchiveSearchResponse {
                        events: vec![HistoryEvent::transcript(2, &visible, context.clone())],
                        next_sequence: Some(2),
                    },
                    other => panic!("unexpected after sequence {other:?}"),
                };
                Ok(response)
            },
        )
        .expect("archive recall should collect");

        assert_eq!(calls, vec![(None, Some(1)), (Some(1), Some(1))]);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.content.as_deref())
                .collect::<Vec<_>>(),
            vec!["archived visible"]
        );
    }

    #[test]
    fn archive_recall_stops_on_non_advancing_sequence() {
        let hidden = observed_state_signal("turn-1", 1_100);
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let mut calls = 0;

        let events = query_projected_archive_history_events(
            HistoryEventQuery {
                session_id: Some("session-1".to_string()),
                after_sequence: Some(1),
                limit: Some(1),
                ..HistoryEventQuery::default()
            },
            1,
            |query| {
                calls += 1;
                assert_eq!(query.after_sequence, Some(1));
                Ok(HistoryArchiveSearchResponse {
                    events: vec![HistoryEvent::transcript(2, &hidden, context.clone())],
                    next_sequence: Some(1),
                })
            },
        )
        .expect("archive recall should stop");

        assert_eq!(calls, 1);
        assert!(events.is_empty());
    }
}
