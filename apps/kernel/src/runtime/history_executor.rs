use crate::error::DaemonError;
use crate::history::{HistoryEventQuery, OperationalHistoryStore, SessionHistoryStore};
use crate::local::{
    AgentUtilityInput, AgentUtilityKind, AgentUtilityOutput, GetPromptInputHistoryRequest,
    LocalDaemonRequest, LocalDaemonResponse, RecordPromptInputHistoryRequest,
    RunAgentUtilityRequest, SemanticRecallMatch, SemanticSearchRecallMode,
    SemanticSearchRecallRequest,
};
use crate::runtime::agent_utility_executor::run_agent_utility;
use crate::runtime::history_requests::{
    execute_prompt_input_history_request as execute_prompt_input_history,
    execute_query_recall_request as execute_query_recall,
    execute_record_prompt_input_history_request as execute_record_prompt_input_history,
    execute_scoped_session_history_blob_content_request,
    execute_scoped_session_history_outline_request, knn_semantic_recall_search,
    recall_query_from_request, recall_query_from_search_request,
    semantic_recall_utility_input_from_search_request,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_history_request(
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetSessionHistoryOutline(request) => {
            let snapshot = runtime_state.session_snapshot(&request.session_id).await?;
            ensure_operational_history_for_outline(
                &history_store,
                &operational_history_store,
                &snapshot,
            )?;
            let agent_imports = snapshot
                .agents()
                .iter()
                .filter_map(|agent| {
                    agent
                        .external_provider_import()
                        .cloned()
                        .map(|import| (agent.id().to_string(), import))
                })
                .collect();
            execute_scoped_session_history_outline_request(
                operational_history_store,
                request,
                agent_imports,
            )
            .await
        }
        LocalDaemonRequest::GetSessionHistoryBlobContent(request) => {
            let snapshot = runtime_state.session_snapshot(&request.session_id).await?;
            let agent_import = snapshot
                .agents()
                .iter()
                .find(|agent| agent.id() == request.agent_id)
                .and_then(|agent| agent.external_provider_import().cloned());
            execute_scoped_session_history_blob_content_request(
                operational_history_store,
                request,
                agent_import,
            )
            .await
        }
        LocalDaemonRequest::GetPromptInputHistory(request) => {
            execute_prompt_input_history_request(operational_history_store, request).await
        }
        LocalDaemonRequest::RecordPromptInputHistory(request) => {
            execute_record_prompt_input_history_request(operational_history_store, request).await
        }
        LocalDaemonRequest::QueryRecall(request) => {
            execute_query_recall_request(
                operational_history_store,
                config_projection,
                recall_query_from_request(request),
            )
            .await
        }
        LocalDaemonRequest::SearchRecall(request) => {
            execute_query_recall_request(
                operational_history_store,
                config_projection,
                recall_query_from_search_request(request),
            )
            .await
        }
        LocalDaemonRequest::SemanticSearchRecall(request) => {
            execute_semantic_search_recall_request(runtime_state, config_projection, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "recall request",
            message: "unsupported recall request".to_string(),
        }),
    }
}

fn ensure_operational_history_for_outline(
    history_store: &SessionHistoryStore,
    operational_history_store: &OperationalHistoryStore,
    session: &crate::session::RuntimeSession,
) -> Result<(), DaemonError> {
    if operational_history_store.legacy_fallback_disabled(session.id())? {
        return Ok(());
    }
    let entries = history_store.load(session)?;
    if entries.is_empty() {
        return Ok(());
    }
    let imported = operational_history_store.append_missing_legacy_transcripts(&entries)?;
    if !imported.is_empty() {
        crate::logging::info_with_fields(
            "history.outline",
            "backfilled legacy session history for outline request",
            serde_json::json!({
                "session_id": session.id(),
                "imported_event_count": imported.len(),
            }),
        );
    }
    Ok(())
}

pub(crate) async fn execute_prompt_input_history_request(
    operational_history_store: OperationalHistoryStore,
    request: GetPromptInputHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    execute_prompt_input_history(operational_history_store, request).await
}

pub(crate) async fn execute_record_prompt_input_history_request(
    operational_history_store: OperationalHistoryStore,
    request: RecordPromptInputHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    execute_record_prompt_input_history(operational_history_store, request).await
}

pub(crate) async fn execute_query_recall_request(
    operational_history_store: OperationalHistoryStore,
    config_projection: &DaemonConfigProjectionStore,
    query: HistoryEventQuery,
) -> Result<LocalDaemonResponse, DaemonError> {
    let archive_config = config_projection.snapshot().user_config.history.archive;
    execute_query_recall(operational_history_store, archive_config, query).await
}

pub(crate) async fn execute_semantic_search_recall_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SemanticSearchRecallRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request.mode.unwrap_or_default() {
        SemanticSearchRecallMode::Knn => {
            execute_knn_semantic_search_recall_request(config_projection, request).await
        }
        SemanticSearchRecallMode::Agent => {
            execute_agent_semantic_search_recall_request(runtime_state, config_projection, request)
                .await
        }
    }
}

async fn execute_knn_semantic_search_recall_request(
    config_projection: &DaemonConfigProjectionStore,
    request: SemanticSearchRecallRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let requested_limit = request.limit.unwrap_or(20).clamp(1, 100);
    let (results, next_cursor, unavailable_reason) =
        execute_knn_semantic_recall_search(config_projection, request, requested_limit).await?;
    Ok(LocalDaemonResponse::SemanticRecallEvents {
        results,
        next_cursor,
        unavailable_reason,
        answer: None,
    })
}

async fn execute_agent_semantic_search_recall_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SemanticSearchRecallRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let Some(session_id) = request.session_id.clone() else {
        return Ok(LocalDaemonResponse::SemanticRecallEvents {
            results: Vec::new(),
            next_cursor: None,
            unavailable_reason: Some(
                "focused-agent semantic recall search requires a session".to_string(),
            ),
            answer: None,
        });
    };
    let Some(agent_id) = runtime_state.focused_agent_id(&session_id).await? else {
        return Ok(LocalDaemonResponse::SemanticRecallEvents {
            results: Vec::new(),
            next_cursor: None,
            unavailable_reason: Some(
                "focused-agent semantic recall search requires a focused agent".to_string(),
            ),
            answer: None,
        });
    };
    let result = run_agent_utility(
        runtime_state,
        config_projection.snapshot().user_config.history.archive,
        RunAgentUtilityRequest {
            session_id,
            agent_id,
            kind: AgentUtilityKind::SemanticRecallSearch,
            input: AgentUtilityInput::SemanticRecallSearch(
                semantic_recall_utility_input_from_search_request(request),
            ),
        },
    )
    .await?;
    let AgentUtilityOutput::SemanticRecallSearch { answer, matches } = result.output else {
        return Err(DaemonError::LocalTransport {
            operation: "semantic recall agent search",
            message: "semantic recall utility returned unexpected output".to_string(),
        });
    };
    Ok(LocalDaemonResponse::SemanticRecallEvents {
        results: matches,
        next_cursor: None,
        unavailable_reason: None,
        answer: Some(answer),
    })
}

async fn execute_knn_semantic_recall_search(
    config_projection: &DaemonConfigProjectionStore,
    request: SemanticSearchRecallRequest,
    requested_limit: usize,
) -> Result<(Vec<SemanticRecallMatch>, Option<String>, Option<String>), DaemonError> {
    let archive_config = config_projection.snapshot().user_config.history.archive;
    knn_semantic_recall_search(archive_config, request, requested_limit).await
}

#[cfg(test)]
mod tests {
    use crate::config::DaemonConfig;
    use crate::history::{
        HistoryEventTurnContext, OperationalHistoryStore, SessionHistoryEntry,
        SessionHistoryEntryKind, SessionHistoryStore,
    };
    use crate::session::{CreateSessionRequest, SessionService};
    use crate::terminal::TerminalOutputKind;

    use super::ensure_operational_history_for_outline;

    #[test]
    fn outline_history_imports_legacy_jsonl_when_operational_history_is_empty() {
        let config = DaemonConfig::for_tests();
        let mut sessions = SessionService::new(&config);
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should create");
        let history_store = SessionHistoryStore::new(config.session_history_root.clone())
            .expect("legacy history should initialize");
        let operational_history_store =
            OperationalHistoryStore::open(config.operational_history_path())
                .expect("operational history should open");
        let legacy_prompt =
            SessionHistoryEntry::user_prompt(session.id(), "attachment-1", "agent-1", "legacy");
        history_store
            .append(&session, &legacy_prompt)
            .expect("legacy history should append");

        ensure_operational_history_for_outline(
            &history_store,
            &operational_history_store,
            &session,
        )
        .expect("legacy history should import");

        let entries = operational_history_store
            .load_session_history_entries(session.id(), Some("agent-1"))
            .expect("operational history should load");
        assert_eq!(entries, vec![legacy_prompt]);
    }

    #[test]
    fn outline_history_imports_missing_legacy_jsonl_after_operational_history_exists() {
        let config = DaemonConfig::for_tests();
        let mut sessions = SessionService::new(&config);
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should create");
        let history_store = SessionHistoryStore::new(config.session_history_root.clone())
            .expect("legacy history should initialize");
        let operational_history_store =
            OperationalHistoryStore::open(config.operational_history_path())
                .expect("operational history should open");
        let external_prompt = SessionHistoryEntry::external_provider_observed(
            session.id(),
            None,
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "external",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(1),
        );
        operational_history_store
            .append_transcript(&external_prompt, HistoryEventTurnContext::default())
            .expect("operational history should append");
        let legacy_output = SessionHistoryEntry::provider_output(
            session.id(),
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some("legacy-output".to_string()),
            "legacy output",
        );
        history_store
            .append(&session, &legacy_output)
            .expect("legacy history should append");

        ensure_operational_history_for_outline(
            &history_store,
            &operational_history_store,
            &session,
        )
        .expect("legacy history should import missing entries");

        let entries = operational_history_store
            .load_session_history_entries(session.id(), Some("agent-1"))
            .expect("operational history should load");
        assert_eq!(entries, vec![external_prompt, legacy_output]);
    }
}
