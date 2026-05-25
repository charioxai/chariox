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
            events,
            next_sequence,
        } => Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "mode": mode,
                "events": events,
                "next_sequence": next_sequence,
            }),
        }),
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
            results,
            next_cursor,
            unavailable_reason,
            answer,
        } => Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "mode": mode,
                "results": results,
                "next_cursor": next_cursor,
                "unavailable_reason": unavailable_reason,
                "answer": answer,
            }),
        }),
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_semantic_recall_response",
            message: format!("unexpected semantic recall response: {other:?}"),
        }),
    }
}
