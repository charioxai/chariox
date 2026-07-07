//! Semantic recall search request mapping and archive-backed search.

use crate::config::UserArchiveHistoryConfig;
use crate::error::DaemonError;
use crate::history::HistoryEventQuery;
use crate::history_archive::{HistoryArchiveClient, HistoryArchiveSemanticSearchResponse};
use crate::local::{
    SemanticRecallMatch, SemanticRecallSearchUtilityInput, SemanticSearchRecallMode,
    SemanticSearchRecallRequest,
};

pub(crate) async fn knn_semantic_recall_search(
    archive_config: UserArchiveHistoryConfig,
    request: SemanticSearchRecallRequest,
    requested_limit: usize,
) -> Result<(Vec<SemanticRecallMatch>, Option<String>, Option<String>), DaemonError> {
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
                Some("semantic recall search is not configured for this kernel".to_string()),
            ));
        }
        let cursor = request.cursor.clone();
        let mut query = history_query_from_semantic_recall_request(request);
        let (results, next_cursor) =
            collect_projected_semantic_recall_matches(requested_limit, cursor, |cursor, limit| {
                query.limit = Some(limit);
                archive_client.semantic_search_events(query.clone(), cursor)
            })?;
        Ok((results, next_cursor, None))
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "semantic search history",
        message: error.to_string(),
    })??;
    Ok(response)
}

pub(crate) fn semantic_recall_utility_input_from_search_request(
    request: SemanticSearchRecallRequest,
) -> SemanticRecallSearchUtilityInput {
    SemanticRecallSearchUtilityInput {
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

pub(crate) fn semantic_recall_request_from_utility_input(
    input: &SemanticRecallSearchUtilityInput,
) -> SemanticSearchRecallRequest {
    SemanticSearchRecallRequest {
        query: input.query.clone(),
        mode: Some(SemanticSearchRecallMode::Knn),
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

fn history_query_from_semantic_recall_request(
    request: SemanticSearchRecallRequest,
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

fn semantic_recall_match_projects_as_result(match_: &SemanticRecallMatch) -> bool {
    match_
        .event
        .to_session_history_entry()
        .is_none_or(|entry| !entry.is_external_provider_observed_state_signal())
}

fn collect_projected_semantic_recall_matches<F>(
    requested_limit: usize,
    initial_cursor: Option<String>,
    mut fetch: F,
) -> Result<(Vec<SemanticRecallMatch>, Option<String>), DaemonError>
where
    F: FnMut(Option<String>, usize) -> Result<HistoryArchiveSemanticSearchResponse, DaemonError>,
{
    let requested_limit = requested_limit.clamp(1, 500);
    let mut cursor = initial_cursor;
    let mut results = Vec::new();
    loop {
        let remaining = requested_limit.saturating_sub(results.len());
        if remaining == 0 {
            break;
        }
        let mut response = fetch(cursor, remaining)?;
        response
            .results
            .retain(semantic_recall_match_projects_as_result);
        results.extend(response.results);
        cursor = response.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    results.truncate(requested_limit);
    Ok((results, cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryEntryKind};

    #[test]
    fn semantic_utility_request_forces_knn_mode_and_preserves_filters() {
        let request =
            semantic_recall_request_from_utility_input(&SemanticRecallSearchUtilityInput {
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

        assert_eq!(request.mode, Some(SemanticSearchRecallMode::Knn));
        assert_eq!(request.cursor, None);
        assert_eq!(request.session_id.as_deref(), Some("session-1"));
        assert_eq!(request.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(request.worktree_path.as_deref(), Some("/repo"));
        assert_eq!(request.kind.as_deref(), Some("terminal_output"));
        assert_eq!(request.limit, Some(12));
    }

    #[test]
    fn semantic_recall_results_hide_external_observer_state_signals() {
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
        let visible_match = SemanticRecallMatch {
            event: crate::history::HistoryEvent::transcript(1, &visible, context.clone()),
            score_millis: Some(10),
            chunk_index: Some(0),
            chunk_text: Some("visible output".to_string()),
            reason: None,
        };
        let hidden_match = SemanticRecallMatch {
            event: crate::history::HistoryEvent::transcript(2, &hidden, context),
            score_millis: Some(11),
            chunk_index: Some(0),
            chunk_text: Some("settled".to_string()),
            reason: None,
        };

        assert!(semantic_recall_match_projects_as_result(&visible_match));
        assert!(!semantic_recall_match_projects_as_result(&hidden_match));
    }

    #[test]
    fn semantic_recall_collection_pages_past_hidden_state_signals() {
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
        let visible_one = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            "visible one",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_200),
        );
        let visible_two = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            "visible two",
            "codex",
            "thread-1",
            Some("turn-2".to_string()),
            Some(2_300),
        );
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let mut calls = Vec::new();

        let (results, next_cursor) = collect_projected_semantic_recall_matches(
            2,
            Some("cursor-0".to_string()),
            |cursor, limit| {
                calls.push((cursor.clone(), limit));
                let response = match cursor.as_deref() {
                    Some("cursor-0") => HistoryArchiveSemanticSearchResponse {
                        results: vec![SemanticRecallMatch {
                            event: crate::history::HistoryEvent::transcript(
                                1,
                                &hidden,
                                context.clone(),
                            ),
                            score_millis: Some(1),
                            chunk_index: Some(0),
                            chunk_text: Some("settled".to_string()),
                            reason: None,
                        }],
                        next_cursor: Some("cursor-1".to_string()),
                    },
                    Some("cursor-1") => HistoryArchiveSemanticSearchResponse {
                        results: vec![
                            SemanticRecallMatch {
                                event: crate::history::HistoryEvent::transcript(
                                    2,
                                    &visible_one,
                                    context.clone(),
                                ),
                                score_millis: Some(2),
                                chunk_index: Some(0),
                                chunk_text: Some("visible one".to_string()),
                                reason: None,
                            },
                            SemanticRecallMatch {
                                event: crate::history::HistoryEvent::transcript(
                                    3,
                                    &visible_two,
                                    context.clone(),
                                ),
                                score_millis: Some(3),
                                chunk_index: Some(0),
                                chunk_text: Some("visible two".to_string()),
                                reason: None,
                            },
                        ],
                        next_cursor: Some("cursor-2".to_string()),
                    },
                    other => panic!("unexpected cursor {other:?}"),
                };
                Ok(response)
            },
        )
        .expect("semantic recall collection should succeed");

        assert_eq!(
            calls,
            vec![
                (Some("cursor-0".to_string()), 2),
                (Some("cursor-1".to_string()), 2)
            ]
        );
        assert_eq!(
            results
                .iter()
                .filter_map(|match_| match_.event.content.as_deref())
                .collect::<Vec<_>>(),
            vec!["visible one", "visible two"]
        );
        assert_eq!(next_cursor.as_deref(), Some("cursor-2"));
    }
}
