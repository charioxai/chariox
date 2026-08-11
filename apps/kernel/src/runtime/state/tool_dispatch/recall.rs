use super::*;

use crate::local::{LocalDaemonResponse, SemanticSearchRecallMode, SemanticSearchRecallRequest};
use crate::runtime::history_executor::{
    execute_query_recall_request, execute_semantic_search_recall_request,
};
use crate::runtime::history_requests::{
    recall_query_from_request, recall_query_from_search_request,
};
use crate::transport::runtime_tools::{
    QueryRecallArgs, RuntimeToolResult, SearchRecallArgs, QUERY_RECALL_TOOL, SEARCH_RECALL_TOOL,
};

const MAX_RECALL_EVENT_CONTENT_BYTES: usize = 16 * 1024;
const MAX_RECALL_RESULT_CONTENT_BYTES: usize = 256 * 1024;

impl KernelRuntimeState {
    pub(super) async fn dispatch_recall_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        match tool_name {
            SEARCH_RECALL_TOOL => {
                let args =
                    serde_json::from_value::<SearchRecallArgs>(arguments).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_search_recall",
                            message: format!("invalid tool arguments: {error}"),
                        }
                    })?;
                self.dispatch_search_recall_tool(provider_run, args).await
            }
            QUERY_RECALL_TOOL => {
                let args =
                    serde_json::from_value::<QueryRecallArgs>(arguments).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_query_recall",
                            message: format!("invalid tool arguments: {error}"),
                        }
                    })?;
                self.dispatch_query_recall_tool(provider_run, args).await
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_recall",
                message: format!("unsupported recall tool `{tool_name}`"),
            }),
        }
    }

    async fn dispatch_search_recall_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        args: SearchRecallArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let limit = args.limit.unwrap_or(20).clamp(1, 50);
        match args.mode.as_deref().unwrap_or("keyword") {
            "keyword" => {
                let request = crate::local::SearchRecallRequest {
                    query: args.query,
                    session_id: scoped_session_id(
                        provider_run,
                        args.scope.as_deref(),
                        args.session_id,
                    )?,
                    agent_id: args.agent_id,
                    provider: args.provider,
                    model: args.model,
                    workflow_id: args.workflow_id,
                    machine_id: None,
                    repo_root: None,
                    worktree_path: None,
                    kind: args.kind,
                    after_sequence: args.after_sequence,
                    limit: Some(limit),
                };
                let query = recall_query_from_search_request(request);
                recall_events_tool_result(
                    execute_query_recall_request(
                        self.owned.operational_history_store.clone(),
                        &self.owned.config_projection,
                        query,
                    )
                    .await?,
                    "keyword",
                )
            }
            "semantic" | "agent" => {
                let mode = if args.mode.as_deref() == Some("agent") {
                    SemanticSearchRecallMode::Agent
                } else {
                    SemanticSearchRecallMode::Knn
                };
                let request = SemanticSearchRecallRequest {
                    query: args.query,
                    mode: Some(mode),
                    session_id: scoped_session_id(
                        provider_run,
                        args.scope.as_deref(),
                        args.session_id,
                    )?,
                    agent_id: args.agent_id,
                    provider: args.provider,
                    model: args.model,
                    workflow_id: args.workflow_id,
                    machine_id: None,
                    repo_root: None,
                    worktree_path: None,
                    kind: args.kind,
                    cursor: args.cursor,
                    limit: Some(limit),
                };
                semantic_recall_events_tool_result(
                    execute_semantic_search_recall_request(
                        self,
                        &self.owned.config_projection,
                        request,
                    )
                    .await?,
                    if mode == SemanticSearchRecallMode::Agent {
                        "agent"
                    } else {
                        "semantic"
                    },
                )
            }
            other => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": format!("mode must be one of: keyword, semantic, agent; got `{other}`")
                }),
            }),
        }
    }

    async fn dispatch_query_recall_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        args: QueryRecallArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let query = recall_query_from_request(crate::local::QueryRecallRequest {
            session_id: scoped_session_id(provider_run, args.scope.as_deref(), args.session_id)?,
            agent_id: args.agent_id,
            provider: args.provider,
            model: args.model,
            workflow_id: args.workflow_id,
            machine_id: None,
            repo_root: None,
            worktree_path: None,
            kind: args.kind,
            text: args.text,
            after_sequence: args.after_sequence,
            before_sequence: args.before_sequence,
            limit: Some(args.limit.unwrap_or(20).clamp(1, 50)),
        });
        recall_events_tool_result(
            execute_query_recall_request(
                self.owned.operational_history_store.clone(),
                &self.owned.config_projection,
                query,
            )
            .await?,
            "query",
        )
    }
}

fn scoped_session_id(
    provider_run: &crate::provider::RuntimeProviderRun,
    scope: Option<&str>,
    requested_session_id: Option<String>,
) -> Result<Option<String>, DaemonError> {
    match scope.unwrap_or("current_session") {
        "current_session" => Ok(Some(
            requested_session_id.unwrap_or_else(|| provider_run.session_id().to_string()),
        )),
        "all" => Ok(requested_session_id),
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_recall_scope",
            message: format!("scope must be one of: current_session, all; got `{other}`"),
        }),
    }
}

fn recall_events_tool_result(
    response: LocalDaemonResponse,
    mode: &str,
) -> Result<RuntimeToolResult, DaemonError> {
    match response {
        LocalDaemonResponse::RecallEvents {
            mut events,
            next_sequence,
        } => {
            bound_recall_events(&mut events);
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "mode": mode,
                    "events": events,
                    "next_sequence": next_sequence,
                }),
            })
        }
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_recall_response",
            message: format!("unexpected recall response: {other:?}"),
        }),
    }
}

fn semantic_recall_events_tool_result(
    response: LocalDaemonResponse,
    mode: &str,
) -> Result<RuntimeToolResult, DaemonError> {
    match response {
        LocalDaemonResponse::SemanticRecallEvents {
            mut results,
            next_cursor,
            unavailable_reason,
            mut answer,
        } => {
            let mut remaining = MAX_RECALL_RESULT_CONTENT_BYTES;
            for result in &mut results {
                bound_recall_event(&mut result.event, &mut remaining);
                bound_optional_text(
                    &mut result.chunk_text,
                    &mut remaining,
                    MAX_RECALL_EVENT_CONTENT_BYTES,
                );
            }
            let mut answer_budget = MAX_RECALL_EVENT_CONTENT_BYTES;
            bound_optional_text(
                &mut answer,
                &mut answer_budget,
                MAX_RECALL_EVENT_CONTENT_BYTES,
            );
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "mode": mode,
                    "results": results,
                    "next_cursor": next_cursor,
                    "unavailable_reason": unavailable_reason,
                    "answer": answer,
                }),
            })
        }
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_semantic_recall_response",
            message: format!("unexpected semantic recall response: {other:?}"),
        }),
    }
}

fn bound_recall_events(events: &mut [crate::history::HistoryEvent]) {
    let mut remaining = MAX_RECALL_RESULT_CONTENT_BYTES;
    for event in events {
        bound_recall_event(event, &mut remaining);
    }
}

fn bound_recall_event(event: &mut crate::history::HistoryEvent, remaining: &mut usize) {
    let Some(content) = event.content.as_ref() else {
        return;
    };
    let original_bytes = content.len();
    let allowed = (*remaining).min(MAX_RECALL_EVENT_CONTENT_BYTES);
    if allowed == 0 {
        event.content = None;
        event.metadata.insert(
            "arroba_content_omitted".to_string(),
            serde_json::Value::Bool(true),
        );
        event.metadata.insert(
            "arroba_original_content_bytes".to_string(),
            serde_json::json!(original_bytes),
        );
        return;
    }
    let bounded = truncate_utf8_bytes(content, allowed);
    *remaining = remaining.saturating_sub(bounded.len());
    if bounded.len() < original_bytes {
        event.metadata.insert(
            "arroba_content_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
        event.metadata.insert(
            "arroba_original_content_bytes".to_string(),
            serde_json::json!(original_bytes),
        );
    }
    event.content = Some(bounded);
}

fn bound_optional_text(value: &mut Option<String>, remaining: &mut usize, item_limit: usize) {
    let Some(text) = value.as_ref() else {
        return;
    };
    let allowed = (*remaining).min(item_limit);
    if allowed == 0 {
        *value = None;
        return;
    }
    let bounded = truncate_utf8_bytes(text, allowed);
    *remaining = remaining.saturating_sub(bounded.len());
    *value = Some(bounded);
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    const MARKER: &str = "\n… [recall content truncated]";
    if max_bytes <= MARKER.len() {
        let mut end = max_bytes;
        while !MARKER.is_char_boundary(end) {
            end -= 1;
        }
        return MARKER[..end].to_string();
    }
    let mut end = max_bytes - MARKER.len();
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARKER}", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(sequence: u64, content: &str) -> crate::history::HistoryEvent {
        crate::history::HistoryEvent::transcript(
            sequence,
            &crate::history::SessionHistoryEntry::provider_output(
                "session-recall-bounds",
                "provider-run-recall-bounds",
                Some("agent-recall-bounds"),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                content,
            ),
            crate::history::HistoryEventTurnContext::default(),
        )
    }

    #[test]
    fn keyword_recall_caps_individual_and_total_content() {
        let mut events = (1..=50)
            .map(|sequence| event(sequence, &"x".repeat(100 * 1024)))
            .collect::<Vec<_>>();

        bound_recall_events(&mut events);

        let retained_bytes = events
            .iter()
            .filter_map(|event| event.content.as_ref())
            .map(String::len)
            .sum::<usize>();
        assert!(retained_bytes <= MAX_RECALL_RESULT_CONTENT_BYTES);
        assert!(events.iter().all(|event| event
            .content
            .as_ref()
            .is_none_or(|content| content.len() <= MAX_RECALL_EVENT_CONTENT_BYTES)));
        assert!(events.iter().any(|event| {
            event.metadata.get("arroba_content_truncated") == Some(&serde_json::Value::Bool(true))
        }));
        assert!(events.iter().any(|event| {
            event.metadata.get("arroba_content_omitted") == Some(&serde_json::Value::Bool(true))
        }));
    }

    #[test]
    fn recall_truncation_preserves_utf8_boundaries() {
        let value = "é".repeat(100);
        let bounded = truncate_utf8_bytes(&value, 31);

        assert!(bounded.len() <= 31);
        assert!(bounded.contains("recall"));
    }
}
