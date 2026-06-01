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
    execute_session_history_blob_content_request, execute_session_history_outline_request,
    knn_semantic_recall_search, recall_query_from_request, recall_query_from_search_request,
    semantic_recall_utility_input_from_search_request,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, SessionHistoryProjectionStore};
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_history_request(
    _history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    _history_projection: SessionHistoryProjectionStore,
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetSessionHistoryOutline(request) => {
            let _ = runtime_state.session_snapshot(&request.session_id).await?;
            execute_session_history_outline_request(operational_history_store, request).await
        }
        LocalDaemonRequest::GetSessionHistoryBlobContent(request) => {
            let _ = runtime_state.session_snapshot(&request.session_id).await?;
            execute_session_history_blob_content_request(operational_history_store, request).await
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
