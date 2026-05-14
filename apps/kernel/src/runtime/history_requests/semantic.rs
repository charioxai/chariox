//! Semantic history search request mapping and archive-backed search.

use crate::config::UserArchiveHistoryConfig;
use crate::error::DaemonError;
use crate::history::HistoryEventQuery;
use crate::history_archive::HistoryArchiveClient;
use crate::local::{
    SemanticHistoryMatch, SemanticHistorySearchUtilityInput, SemanticSearchHistoryMode,
    SemanticSearchHistoryRequest,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
